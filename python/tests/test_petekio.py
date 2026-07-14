"""End-to-end tests for the petekio PyO3 bindings, exercised against the
committed fixtures under ``tests/fixtures/``. Run with ``pytest`` against an
installed wheel.
"""

import math
import warnings
from pathlib import Path

import pytest

import petekio

# The fixtures live at the repo root (two levels up from this file:
# python/tests/ -> repo root).
FIXTURES = Path(__file__).resolve().parents[2] / "tests" / "fixtures"
IRAP = str(FIXTURES / "simple.irap")
LAS = str(FIXTURES / "sample.las")
TOPS = str(FIXTURES / "tops.csv")
POINTS_GEOJSON = str(FIXTURES / "points.geojson")
SQUARE_GEOJSON = str(FIXTURES / "square.geojson")
WELL_DIR = str(FIXTURES / "wells" / "15_9-A1")


# --------------------------------------------------------------------------
# Surface
# --------------------------------------------------------------------------


def _write_irap_surface(tmp_path, name, geom, values):
    """Write a small synthetic grid in the committed IRAP-classic convention."""
    yinc = -geom.yinc if geom.yflip else geom.yinc
    xmax = geom.xori + (geom.ncol - 1) * geom.xinc
    ymax = geom.yori + (geom.nrow - 1) * geom.yinc
    tokens = ["9999900.0" if math.isnan(v) else repr(v) for v in values]
    path = tmp_path / f"{name}.irap"
    path.write_text(
        "\n".join(
            [
                f"-996 {geom.nrow} {geom.xinc} {yinc}",
                f"{geom.xori} {xmax} {geom.yori} {ymax}",
                f"{geom.ncol} {geom.rotation_deg} {geom.xori} {geom.yori}",
                "0 0 0 0 0 0 0",
                " ".join(tokens),
            ]
        )
        + "\n"
    )
    return petekio.Surface.load_irap_classic(str(path))


def _plane_surface(tmp_path, name, geom, gx, gy, intercept=100.0):
    values = []
    for j in range(geom.nrow):
        for i in range(geom.ncol):
            x, y = geom.node_xy(i, j)
            values.append(intercept + gx * x + gy * y)
    return _write_irap_surface(tmp_path, name, geom, values)


def test_surface_load_and_geometry():
    s = petekio.Surface.load_irap_classic(IRAP)
    assert s.ncol == 3
    assert s.nrow == 4
    g = s.geometry
    assert g.ncol == 3 and g.nrow == 4
    assert g.xinc == 50.0 and g.yinc == 25.0
    # bbox is finite
    b = s.bbox()
    assert math.isfinite(b.xmin) and math.isfinite(b.xmax)


def test_surface_edge_matches_geometry_edge():
    g = petekio.GridGeometry(0.0, 0.0, 10.0, 10.0, 3, 3)
    s = petekio.Surface.constant(g, 5.0)
    assert math.isclose(s.edge.area(), 400.0, abs_tol=1e-9)
    assert math.isclose(s.geometry.edge.area(), s.edge.area(), abs_tol=1e-9)


def test_surface_sample_and_stats():
    s = petekio.Surface.load_irap_classic(IRAP)
    st = s.stats()
    # 12 nodes, one undefined (9999900 sentinel) -> 11 defined.
    assert st.count == 11
    assert st.min <= st.mean <= st.max
    assert math.isclose(st.percentile(0.5), st.p50, rel_tol=1e-9)
    # Sample at the origin node returns a finite value.
    v = s.sample(g_origin_x(s), g_origin_y(s))
    assert v is not None and math.isfinite(v)
    # Far outside -> None.
    assert s.sample(1e9, 1e9) is None


def g_origin_x(s):
    return s.geometry.node_xy(0, 0)[0]


def g_origin_y(s):
    return s.geometry.node_xy(0, 0)[1]


def test_surface_scalar_operators():
    g = petekio.GridGeometry(0.0, 0.0, 10.0, 10.0, 3, 3)
    s = petekio.Surface.constant(g, 5.0)
    assert (s + 10.0).stats().mean == 15.0
    assert "surface.constant(value=5" in s.history()[0]
    assert any("surface.add_scalar" in h for h in (s + 10.0).history())
    assert (s - 2.0).stats().mean == 3.0
    assert (s * 2.0).stats().mean == 10.0
    assert (s / 5.0).stats().mean == 1.0
    # reflected scalar ops
    assert (10.0 + s).stats().mean == 15.0
    assert (2.0 * s).stats().mean == 10.0
    assert (20.0 - s).stats().mean == 15.0


def test_surface_surface_operators_and_mismatch():
    g = petekio.GridGeometry(0.0, 0.0, 10.0, 10.0, 3, 3)
    top = petekio.Surface.constant(g, 100.0)
    base = petekio.Surface.constant(g, 130.0)
    thick = (base - top).clamp_min(0.0)
    assert thick.stats().mean == 30.0
    # named forms agree with operators
    minus = base.minus(top)
    assert minus.stats().mean == 30.0
    assert any("rhs.surface.constant(value=100" in h for h in minus.history())
    assert any("surface.minus(surface)" in h for h in minus.history())
    # thickness computes base - top in both unbound and instance forms.
    unbound = petekio.Surface.thickness(top, base)
    assert unbound.stats().mean == 30.0
    assert any("surface.thickness(clamp_zero=false)" in h for h in unbound.history())
    # Normal instance method and unbound class form are equivalent.
    assert top.thickness(base).stats().mean == 30.0
    assert top.thickness(base, clamp_zero=True).stats().mean == 30.0
    assert petekio.Surface.thickness(top, base, clamp_zero=True).stats().mean == 30.0
    assert base.thickness(top).stats().mean == -30.0
    assert base.thickness(top, clamp_zero=True).stats().mean == 0.0
    # geometry mismatch raises
    other = petekio.Surface.constant(petekio.GridGeometry(0.0, 0.0, 10.0, 10.0, 4, 4), 1.0)
    with pytest.raises(ValueError):
        _ = top + other
    with pytest.raises(ValueError):
        top.thickness(other)


def test_surface_elementwise_math():
    g = petekio.GridGeometry(0.0, 0.0, 10.0, 10.0, 2, 2)
    s = petekio.Surface.constant(g, 100.0)
    assert math.isclose(s.log10().stats().mean, 2.0)
    assert math.isclose(s.sqrt().stats().mean, 10.0)
    assert math.isclose(s.clamp(0.0, 50.0).stats().mean, 50.0)
    assert math.isclose(s.powf(2.0).stats().mean, 10000.0)
    assert math.isclose((s.ln()).exp().stats().mean, 100.0, rel_tol=1e-9)


def test_surface_volumetrics():
    g = petekio.GridGeometry(0.0, 0.0, 10.0, 10.0, 2, 2)
    s = petekio.Surface.constant(g, 10.0)
    base = petekio.Surface.constant(g, 0.0)
    # |10-0| * cell(100) * 4 nodes = 4000
    assert s.volume_between(base) == 4000.0
    # area below/above
    assert s.area_below(10.0) == 400.0
    assert s.area_above(10.0) == 400.0
    assert s.area_below(5.0) == 0.0
    h = s.hypsometry()
    assert len(h) == 4
    assert h[-1][1] == 400.0


def test_surface_resample_rotated_is_unsupported():
    # The committed IRAP fixture is rotated (rotation_deg == 30). The shared
    # resample kernel is axis-aligned-only, so resampling a rotated geometry
    # raises LOUDLY rather than returning a silently-untested answer (pending
    # suite task_suite_grid_rotation). Real IRAP/RMS exports CAN be rotated.
    s = petekio.Surface.load_irap_classic(IRAP)
    assert s.geometry.rotation_deg == 30.0
    with pytest.raises(ValueError):
        s.resample(s.geometry)


def test_surface_resample_axis_aligned():
    # The supported path: an axis-aligned surface resamples onto a finer
    # axis-aligned lattice. A constant field stays constant everywhere inside.
    g = petekio.GridGeometry(0.0, 0.0, 10.0, 10.0, 3, 3)
    s = petekio.Surface.constant(g, 7.0)
    fine = petekio.GridGeometry(0.0, 0.0, 5.0, 5.0, 5, 5)
    r = s.resample(fine)
    assert r.ncol == 5 and r.nrow == 5
    assert math.isclose(r.stats().mean, 7.0, rel_tol=1e-12)


def test_surface_attr_access():
    g = petekio.GridGeometry(0.0, 0.0, 10.0, 10.0, 2, 2)
    s = petekio.Surface.constant(g, 1.0)
    seismic = petekio.Surface.constant(g, 7.0)
    s.set_attr("seismic", seismic)
    assert any("surface.set_attr(name=seismic)" in h for h in s.history())
    assert "seismic" in s.attr
    assert s.attr_names() == ["seismic"]
    # indexed access promotes to a Surface
    promoted = s.attr["seismic"]
    assert promoted.stats().mean == 7.0
    assert any("surface.as_attr_surface(name=seismic)" in h for h in promoted.history())
    # ln() on the promoted attribute works (returns a Surface, not ndarray)
    assert math.isclose(s.attr["seismic"].ln().stats().mean, math.log(7.0))
    # call form too
    assert s.attr("seismic").stats().mean == 7.0
    with pytest.raises(KeyError):
        _ = s.attr["missing"]


def test_surface_attribute_metadata_is_canonical_and_preserved_on_replace():
    g = petekio.GridGeometry(0.0, 0.0, 10.0, 10.0, 2, 2)
    s = petekio.Surface.constant(g, 1.0)
    values = petekio.Surface.constant(g, 0.2)
    metadata = {
        "id": "porosity",
        "label": "Porosity",
        "kind": "continuous",
        "units": "v/v",
        "codes": None,
    }
    s.set_attr("porosity", values, metadata=metadata)
    assert s.attr_metadata("porosity") == metadata
    assert s.attr["porosity"].primary_metadata == metadata

    s.set_attr("porosity", petekio.Surface.constant(g, 0.3))
    assert s.attr_metadata("porosity") == metadata
    with pytest.raises(ValueError, match="must match lane name"):
        s.set_attr("porosity", values, metadata={"id": "other"})
    for invalid in (
        {"id": " "},
        {"id": "porosity", "label": "\t"},
        {"id": "porosity", "units": " \n"},
    ):
        with pytest.raises(ValueError, match="non-empty after trimming"):
            s.set_attr("porosity", values, metadata=invalid)
    with pytest.raises(TypeError, match="unknown attribute metadata key 'unit'"):
        s.set_attr("porosity", values, metadata={"id": "porosity", "unit": "v/v"})
    with pytest.raises(TypeError, match="unknown attribute code record key 'colour'"):
        s.set_attr(
            "facies",
            values,
            metadata={
                "id": "facies",
                "kind": "categorical",
                "codes": {"1": {"colour": "#EDA100"}},
            },
        )


def test_surface_smooth_dip_and_extrapolate(tmp_path):
    rotated_flip = petekio.GridGeometry(
        100.0, 200.0, 2.0, 3.0, 4, 5, rotation_deg=37.0, yflip=True
    )
    gx, gy = 0.2, -0.1
    plane = _plane_surface(tmp_path, "rotated_plane", rotated_flip, gx, gy)
    plane.source_lane = petekio.Surface.constant(rotated_flip, 1.0)

    angle = plane.dip_angle()
    azimuth = plane.dip_azimuth()
    expected_angle = math.degrees(math.atan(math.hypot(gx, gy)))
    expected_azimuth = math.degrees(math.atan2(-gx, -gy)) % 360.0
    assert math.isclose(angle.stats().min, expected_angle, abs_tol=1e-10)
    assert math.isclose(angle.stats().max, expected_angle, abs_tol=1e-10)
    assert math.isclose(azimuth.stats().min, expected_azimuth, abs_tol=1e-10)
    assert math.isclose(azimuth.stats().max, expected_azimuth, abs_tol=1e-10)
    assert angle.geometry.rotation_deg == 37.0 and angle.geometry.yflip
    assert angle.attr_names() == [] and azimuth.attr_names() == []
    assert "surface.dip_angle()" in angle.history()[-1]
    assert "surface.dip_azimuth()" in azimuth.history()[-1]

    cardinal = [
        (0.0, -1.0, 0.0),
        (-1.0, 0.0, 90.0),
        (0.0, 1.0, 180.0),
        (1.0, 0.0, 270.0),
    ]
    axis_geom = petekio.GridGeometry(0.0, 0.0, 1.0, 1.0, 3, 3)
    for n, (east_gradient, north_gradient, expected) in enumerate(cardinal):
        direction = _plane_surface(
            tmp_path, f"cardinal_{n}", axis_geom, east_gradient, north_gradient
        ).dip_azimuth()
        assert math.isclose(direction.stats().mean, expected, abs_tol=1e-12)

    flat = petekio.Surface.constant(axis_geom, 7.0)
    assert flat.dip_angle().stats().mean == 0.0
    assert flat.dip_azimuth().stats().count == 0

    holes = [7.0] * 9
    holes[4] = math.nan
    with_hole = _write_irap_surface(tmp_path, "constant_hole", axis_geom, holes)
    with_hole.overlay = petekio.Surface.constant(axis_geom, 3.0)
    smoothed = with_hole.smooth()
    assert smoothed.stats().count == 8
    assert smoothed.attr_names() == []
    assert "surface.smooth(radius=1)" in smoothed.history()[-1]
    assert with_hole.dip_angle().stats().count < 8

    for method in ("nearest", "idw", "min_curvature"):
        filled = with_hole.extrapolate(method)
        assert filled.stats().count == 9
        assert math.isclose(filled.stats().mean, 7.0, abs_tol=1e-8)
        assert filled.attr_names() == []
        assert "surface.extrapolate" in filled.history()[-1]

    all_nan = petekio.Surface.constant(axis_geom, math.nan)
    with pytest.raises(ValueError, match="requires at least one finite source node"):
        all_nan.extrapolate()
    with pytest.raises(TypeError, match="unknown grid method"):
        with_hole.extrapolate("kriging")


def test_surface_attribute_assignment_is_typed_geometry_safe_and_replaceable():
    g = petekio.GridGeometry(0.0, 0.0, 10.0, 10.0, 2, 2)
    s = petekio.Surface.constant(g, 1.0)

    # Assignment is ergonomic sugar for set_attr; replacement keeps one lane.
    s.thickness = petekio.Surface.constant(g, 7.0)
    assert s.attr["thickness"].stats().mean == 7.0
    s.thickness = petekio.Surface.constant(g, 9.0)
    assert s.attr_names() == ["thickness"]
    assert s.attr["thickness"].stats().mean == 9.0

    # The instance/unbound operation is not shadowed by the attribute lane.
    assert petekio.Surface.thickness(s, s).stats().mean == 0.0
    assert s.thickness(s).stats().mean == 0.0

    with pytest.raises(TypeError):
        s.invalid = [1.0, 2.0, 3.0, 4.0]

    wrong_shape = petekio.Surface.constant(
        petekio.GridGeometry(0.0, 0.0, 10.0, 10.0, 3, 2), 2.0
    )
    with pytest.raises(ValueError):
        s.set_attr("wrong_shape", wrong_shape)

    different_geometries = [
        petekio.GridGeometry(100.0, 0.0, 10.0, 10.0, 2, 2),
        petekio.GridGeometry(0.0, 100.0, 10.0, 10.0, 2, 2),
        petekio.GridGeometry(0.0, 0.0, 20.0, 10.0, 2, 2),
        petekio.GridGeometry(0.0, 0.0, 10.0, 20.0, 2, 2),
        petekio.GridGeometry(0.0, 0.0, 10.0, 10.0, 2, 2, rotation_deg=30.0),
        petekio.GridGeometry(0.0, 0.0, 10.0, 10.0, 2, 2, yflip=True),
    ]
    for different in different_geometries:
        rhs = petekio.Surface.constant(different, 2.0)
        with pytest.raises(ValueError):
            s.set_attr("same_shape", rhs)
        with pytest.raises(ValueError):
            s.same_shape = rhs


def test_stats_fields():
    s = petekio.Surface.load_irap_classic(IRAP)
    st = s.stats()
    for f in ("count", "mean", "min", "max", "std", "sum", "p10", "p50", "p90"):
        getattr(st, f)  # read-only attributes exist
    assert "Stats(" in repr(st)


# --------------------------------------------------------------------------
# PointSet / PolygonSet
# --------------------------------------------------------------------------


def test_pointset_geojson():
    p = petekio.PointSet.load_geojson(POINTS_GEOJSON)
    assert len(p) == 3
    assert p.xy() == [(0.0, 0.0), (10.0, 0.0), (0.0, 10.0)]
    assert p.xyz() == [(0.0, 0.0, 10.0), (10.0, 0.0, 20.0), (0.0, 10.0, 30.0)]
    poro = p.attr("poro")
    assert poro == [0.10, 0.20, 0.30]
    assert p.attr("missing") is None
    st = p.stats("poro")
    assert st.count == 3
    assert math.isclose(st.mean, 0.20)
    b = p.bbox()
    assert b.xmin == 0.0 and b.xmax == 10.0
    # nearest to (9, 1) -> index 1 (the point at (10, 0))
    assert p.nearest(9.0, 1.0) == 1


def test_pointset_column_math_and_assignment():
    p = petekio.PointSet.load_geojson(POINTS_GEOJSON)
    assert "points.load_geojson" in p.history()[0]
    assert p.z.values() == [10.0, 20.0, 30.0]
    shifted = p.z + 2.0
    assert shifted.values() == [12.0, 22.0, 32.0]
    p.depth_plus_y = p.z + p.y
    assert p.attr("depth_plus_y") == [10.0, 20.0, 40.0]
    p.set_attr("double_poro", p.poro * 2.0)
    assert any("points.set_attr(name=double_poro)" in h for h in p.history())
    assert p.attr_names() == ["poro", "depth_plus_y", "double_poro"]
    assert p.attr("double_poro") == [0.2, 0.4, 0.6]
    with pytest.raises(AttributeError):
        p.z = shifted
    with pytest.raises(TypeError):
        _ = p + p


def test_pointset_to_surface():
    p = petekio.PointSet.load_geojson(POINTS_GEOJSON)
    g = petekio.GridGeometry(0.0, 0.0, 5.0, 5.0, 3, 3)
    surf = p.to_surface(g, "idw")
    assert surf.ncol == 3 and surf.nrow == 3
    assert "points.load_geojson" in surf.history()[0]
    assert any("points.to_surface(method=InverseDistance)" in h for h in surf.history())
    # nearest method also valid
    surf2 = p.to_surface(g, "nearest")
    assert surf2.stats().count > 0
    with pytest.raises(TypeError):
        p.to_surface(g, "bogus")


def test_anonymous_objects_carry_no_dataset_name():
    p = petekio.PointSet.from_xyz([0.0, 1.0], [0.0, 1.0], [0.0, 1.0])
    assert p.name is None
    g = petekio.GridGeometry(0.0, 0.0, 5.0, 5.0, 3, 3)
    assert g.name is None


def test_pointset_infer_geometry_and_edge_options():
    source = petekio.GridGeometry(456123.5, 6712345.25, 37.0, 83.0, 5, 4, 27.5)
    x, y, z = [], [], []
    for j in range(source.nrow):
        for i in range(source.ncol):
            xi, yi = source.node_xy(i, j)
            x.append(xi)
            y.append(yi)
            z.append(1000.0 + i + j)

    p = petekio.PointSet.from_xyz(x, y, z)
    geom = p.infer_geometry(tolerance=1e-6, edge="full_rect")
    assert math.isclose(geom.xori, source.xori, abs_tol=1e-6)
    assert math.isclose(geom.yori, source.yori, abs_tol=1e-6)
    assert math.isclose(geom.xinc, source.xinc, abs_tol=1e-9)
    assert math.isclose(geom.yinc, source.yinc, abs_tol=1e-9)
    assert geom.ncol == source.ncol
    assert geom.nrow == source.nrow
    assert math.isclose(geom.rotation_deg, source.rotation_deg, abs_tol=1e-9)
    assert math.isclose(geom.edge.area(), (4 * 37.0) * (3 * 83.0), abs_tol=1e-6)

    hull_geom = p.infer_geometry(tolerance=1e-6, edge="convex_hull")
    assert math.isclose(hull_geom.edge.area(), geom.edge.area(), abs_tol=1e-6)
    assert petekio.GridGeometry(0.0, 0.0, 10.0, 10.0, 3, 3).edge.area() == 400.0


def test_earthvision_pointset_infer_geometry_uses_column_row(tmp_path):
    path = tmp_path / "petrel_surface.EarthVisionGrid"
    path.write_text(
        """# Type: scattered data
# Field: 1 x
# Field: 2 y
# Field: 3 z meters
# Field: 4 column
# Field: 5 row
# Grid_size: 3 x 2
# End:
100.0 200.0 -50.0 1 1
110.0 200.0 -51.0 2 1
110.0 200.0 -51.0 3 1
100.0 210.0 -52.0 1 2
110.0 210.0 -53.0 2 2
120.0 210.0 -54.0 3 2
""",
        encoding="utf-8",
    )
    p = petekio.PointSet.load_earthvision_grid(str(path))
    assert p.attr("column") == [1.0, 2.0, 3.0, 1.0, 2.0, 3.0]
    assert p.attr("row") == [1.0, 1.0, 1.0, 2.0, 2.0, 2.0]

    geom = p.infer_geometry(tolerance=1e-3, edge="convex_hull")
    assert geom.ncol == 3
    assert geom.nrow == 2
    assert math.isclose(geom.xinc, 10.0, abs_tol=1e-9)
    assert math.isclose(geom.yinc, 10.0, abs_tol=1e-9)


def test_earthvision_structured_surface_preserves_null_xy(tmp_path):
    path = tmp_path / "null_surface.EarthVisionGrid"
    path.write_text(
        """# Type: scattered data
# Grid_size: 2 x 2
# Null_value: 1.0e30
# End:
0 0 10 1 1
10 0 11 2 1
0 10 1.0e30 1 2
10 10 13 2 2
""",
        encoding="utf-8",
    )
    surface = petekio.StructuredMeshSurface.load_earthvision_grid(str(path))
    assert (surface.ncol, surface.nrow) == (2, 2)
    assert surface.node_xy(0, 1) == (0.0, 10.0)
    assert math.isnan(surface.z(0, 1))
    assert len(surface.to_points()) == 4

    # Deprecated compatibility remains a finite-only point view.
    assert len(petekio.PointSet.load_earthvision_grid(str(path))) == 3


def _l_shaped_lattice():
    x, y, z = [], [], []
    for j in range(4):
        for i in range(4):
            if i <= 1 or j <= 1:
                x.append(float(i))
                y.append(float(j))
                z.append(100.0 + i + j)
    return x, y, z


def test_pointset_occupied_edge_follows_topology_cells():
    x, y, z = _l_shaped_lattice()
    col = [int(v) + 1 for v in x]
    row = [int(v) + 1 for v in y]

    p = petekio.PointSet.from_xyz(x, y, z)
    p.column = col
    p.row = row

    occupied = p.infer_geometry(tolerance=1e-6, edge="occupied")
    hull = p.infer_geometry(tolerance=1e-6, edge="convex_hull")
    full_rect = p.infer_geometry(tolerance=1e-6, edge="full_rect")
    default = p.infer_geometry(tolerance=1e-6)

    # `occupied` tracks the L; the bounding rectangle over-claims it.
    assert math.isclose(occupied.edge.area(), 5.0, abs_tol=1e-12)
    assert math.isclose(full_rect.edge.area(), 9.0, abs_tol=1e-12)
    assert hull.edge.area() > occupied.edge.area()
    # full_rect is the default.
    assert math.isclose(default.edge.area(), full_rect.edge.area(), abs_tol=1e-12)


def test_pointset_occupied_edge_without_topology_matches_topology_footprint():
    # The coordinate path derives each point's lattice index anyway, so it must agree
    # with the topology path on the footprint. (It used to triangulate and answer 5.5.)
    x, y, z = _l_shaped_lattice()
    p = petekio.PointSet.from_xyz(x, y, z)

    occupied = p.infer_geometry(tolerance=1e-6, edge="occupied")
    full_rect = p.infer_geometry(tolerance=1e-6, edge="full_rect")

    assert math.isclose(occupied.edge.area(), 5.0, abs_tol=1e-12)
    assert math.isclose(full_rect.edge.area(), 9.0, abs_tol=1e-12)


def test_pointset_occupied_and_full_rect_agree_on_full_lattice():
    x, y, z = [], [], []
    for j in range(4):
        for i in range(5):
            x.append(i * 10.0)
            y.append(j * 25.0)
            z.append(1.0)
    p = petekio.PointSet.from_xyz(x, y, z)

    occupied = p.infer_geometry(tolerance=1e-9, edge="occupied")
    full_rect = p.infer_geometry(tolerance=1e-9, edge="full_rect")

    expected = (4 * 10.0) * (3 * 25.0)
    assert math.isclose(occupied.edge.area(), expected, rel_tol=1e-9)
    assert math.isclose(full_rect.edge.area(), expected, rel_tol=1e-9)


@pytest.mark.parametrize("removed", ["concave_hull", "alpha", "outer", "trimesh", "tin"])
def test_pointset_removed_geometry_edges_raise(removed):
    x, y, z = _l_shaped_lattice()
    p = petekio.PointSet.from_xyz(x, y, z)
    with pytest.raises(ValueError, match="has been removed"):
        p.infer_geometry(tolerance=1e-6, edge=removed)


def _curvilinear_grid(ncol, nrow, skip_corner=True):
    """~50 m cell, rotated 30 deg, gently swelling and bowed, with a missing corner."""
    c, s = math.cos(math.radians(30.0)), math.sin(math.radians(30.0))
    x, y, z = [], [], []
    for j in range(nrow):
        for i in range(ncol):
            if skip_corner and i >= ncol - 2 and j >= nrow - 2:
                continue
            u = 50.0 * i * (1.0 + 0.004 * i)
            v = 50.0 * j + 0.007 * i * i
            x.append(1000.0 + u * c - v * s)
            y.append(2000.0 + u * s + v * c)
            z.append(-1800.0 - i - j)
    return x, y, z


def test_detect_topology_labels_a_curvilinear_grid_and_round_trips():
    x, y, z = _curvilinear_grid(9, 7)
    p = petekio.PointSet.from_xyz(x, y, z)
    assert p.attr("column") is None  # no topology in the input

    pts, report = p.detect_topology()
    assert report.verified
    assert pts is not None
    assert report.assigned == report.distinct_nodes == len(x)
    assert report.blocks == 1 and report.largest_block == len(x)
    assert report.conflicts == 0
    assert report.stalled_frontier == 0
    # the fixture swells along i, so its modal i-step really is a little above 50 m
    assert 50.0 <= report.detected_cell_i < 54.0
    assert abs(report.detected_cell_j - 50.0) < 1.5

    # the labels are what let the mesh be built, and nothing moved
    with pytest.warns(UserWarning, match="StructuredShell geometry"):
        shell = pts.infer_geometry(tolerance=1e-3)
    assert shell.kind == "structured_shell"
    back = pts.to_structured_surface().to_points()
    assert sorted(zip(x, y, z)) == sorted(back.xyz())


@pytest.mark.parametrize("xinc,yinc", [(50.0, 50.0), (50.0, 25.0), (25.0, 50.0), (20.0, 200.0)])
def test_detect_topology_handles_anisotropic_cells(xinc, yinc):
    # A grid's two increments need not agree; a 50 x 25 m Petrel cell is ordinary.
    c, s = math.cos(math.radians(30.0)), math.sin(math.radians(30.0))
    x, y, z = [], [], []
    for j in range(10):
        for i in range(12):
            u, v = xinc * i, yinc * j
            x.append(1000.0 + u * c - v * s)
            y.append(2000.0 + u * s + v * c)
            z.append(-1800.0 - i - j)
    pts, report = petekio.PointSet.from_xyz(x, y, z).detect_topology()
    assert report.verified, report
    assert pts is not None
    got = sorted((report.detected_cell_i, report.detected_cell_j))
    assert got == pytest.approx(sorted((xinc, yinc)), abs=1e-6)


def test_detect_topology_refuses_to_walk_across_a_fault():
    c, s = math.cos(math.radians(30.0)), math.sin(math.radians(30.0))
    x, y, z = [], [], []

    def node(i, j):
        u, v = 50.0 * i, 50.0 * j
        return 1000.0 + u * c - v * s, 2000.0 + u * s + v * c

    for j in range(8):
        for i in range(6):
            px, py = node(i, j)
            x.append(px); y.append(py); z.append(-1800.0)
    for j in range(8):
        for i in range(8, 14):
            px, py = node(i, j)
            x.append(px + 30.0); y.append(py + 25.0); z.append(-1900.0)

    p = petekio.PointSet.from_xyz(x, y, z)
    pts, report = p.detect_topology()
    assert not report.verified
    assert pts is None, "an unverified detection must not hand back labels"
    # The walk re-seeds where it stalls: every node is labelled, but in >1 block, and a
    # structured mesh has only one (column, row) space.
    assert report.assigned == report.distinct_nodes
    assert report.blocks >= 2

    # ...and the TIN fallback keeps both blocks without bridging the throw.
    tin = p.to_tri_surface()
    assert tin.components == 2
    assert tin.n_points > 6 * 8


def test_detect_topology_coincident_nodes():
    x, y, z = _curvilinear_grid(7, 6)
    # same XY, same z: a fault-collapsed pair — harmless, drop one
    p = petekio.PointSet.from_xyz(x + [x[10]], y + [y[10]], z + [z[10]])
    pts, report = p.detect_topology()
    assert report.coincident_dropped == 1
    assert report.coincident_ambiguous == 0
    assert report.verified and pts is not None

    # same XY, different z: two nodes at one place — refuse
    p = petekio.PointSet.from_xyz(x + [x[10]], y + [y[10]], z + [z[10] + 25.0])
    pts, report = p.detect_topology()
    assert report.coincident_ambiguous == 1
    assert not report.verified
    assert pts is None


def _rotated_lattice(ncol, nrow, xinc, yinc, az):
    c, s = math.cos(math.radians(az)), math.sin(math.radians(az))
    x, y, z = [], [], []
    for j in range(nrow):
        for i in range(ncol):
            u, v = xinc * i, yinc * j
            x.append(1000.0 + u * c - v * s)
            y.append(2000.0 + u * s + v * c)
            z.append(-1800.0)
    return x, y, z


def test_tri_surface_triangulates_a_grid_into_one_sheet():
    x, y, z = _rotated_lattice(9, 7, 50.0, 50.0, 25.0)
    tin = petekio.PointSet.from_xyz(x, y, z).to_tri_surface()
    assert tin.kind == "tri_surface"
    assert tin.n_points == len(x)
    assert tin.n_triangles == 2 * 8 * 6
    assert len(tin.edge.rings()) == 1
    # the vertices are the input points, unmoved
    assert sorted(tin.points()) == sorted(zip(x, y, z))


def test_tri_surface_handles_anisotropic_cells():
    # A 50 x 20 cell has a diagonal longer than two short steps, so no world-unit
    # max link can work. The normalized grid frame makes the cell a unit square.
    x, y, z = _rotated_lattice(9, 7, 50.0, 20.0, 40.0)
    tin = petekio.PointSet.from_xyz(x, y, z).to_tri_surface()
    assert tin.n_triangles == 2 * 8 * 6
    assert len(tin.edge.rings()) == 1


def _faulted_blocks():
    x, y, z = [], [], []
    for j in range(9):
        for i in range(6):
            x.append(50.0 * i)
            y.append(50.0 * j)
            z.append(-1800.0)
    for j in range(9):
        for i in range(8, 14):
            x.append(50.0 * i + 20.0)
            y.append(50.0 * j + 25.0)
            z.append(-1900.0)
    return x, y, z


def test_infer_geometry_default_bridge_closes_fringe_and_explicit_none_is_strict():
    # One point 2.5 cells beyond a regular boundary represents the short fringe
    # produced when an export's edge does not close on the recovered lattice.
    x, y, z = _rotated_lattice(9, 7, 50.0, 50.0, 0.0)
    x.append(1000.0 + 8.0 * 50.0 + 125.0)
    y.append(2000.0 + 3.0 * 50.0)
    z.append(-1800.0)
    p = petekio.PointSet.from_xyz(x, y, z)

    with pytest.warns(UserWarning, match="MeshShell fallback"):
        default = p.infer_geometry(tolerance=1e-3)
    with pytest.warns(UserWarning, match="MeshShell fallback"):
        strict = p.infer_geometry(tolerance=1e-3, max_bridge=None)

    assert default.kind == "mesh_shell"
    assert default.n_nodes == 64
    assert default.components == 1
    assert strict.n_nodes == 63
    assert p.to_tri_surface().n_points == 63


def test_infer_geometry_max_bridge_closes_the_fault_seam():
    x, y, z = _faulted_blocks()
    p = petekio.PointSet.from_xyz(x, y, z)
    with pytest.warns(UserWarning, match="MeshShell fallback"):
        strict = p.infer_geometry(tolerance=1e-3, max_bridge=None)
    assert strict.kind == "mesh_shell"
    assert strict.components == 2
    with pytest.warns(UserWarning, match="MeshShell fallback"):
        bridged = p.infer_geometry(tolerance=1e-3, max_bridge=4.0)
    assert bridged.components == 1
    with pytest.raises(ValueError, match="max_bridge"):
        p.to_tri_surface(max_bridge=1.0)


def test_infer_geometry_fallback_is_loud_and_controllable():
    x, y, z = _faulted_blocks()
    p = petekio.PointSet.from_xyz(x, y, z)

    # Default: the MeshShell fallback fires a UserWarning naming the fit failure.
    with pytest.warns(UserWarning, match="no regular lattice fits these points"):
        mesh = p.infer_geometry(tolerance=1e-3)
    assert mesh.kind == "mesh_shell"

    # fallback="error" raises instead of falling back.
    with pytest.raises(ValueError, match='fallback="error"'):
        p.infer_geometry(tolerance=1e-3, fallback="error")

    # Unknown fallback tokens are rejected loudly.
    with pytest.raises(ValueError, match="unknown geometry fallback"):
        p.infer_geometry(tolerance=1e-3, fallback="bogus")

    # Legacy spelling remains accepted but is explicitly deprecated; the
    # returned object follows the geometry-only contract.
    with warnings.catch_warnings(record=True) as caught:
        warnings.simplefilter("always")
        legacy = p.infer_geometry(tolerance=1e-3, fallback="tri")
    assert legacy.kind == "mesh_shell"
    assert any(issubclass(w.category, DeprecationWarning) for w in caught)
    assert any(issubclass(w.category, UserWarning) for w in caught)


def test_infer_geometry_reports_both_failures_when_fallback_also_fails():
    # Collinear points: no regular lattice fits, and the cloud is genuinely
    # degenerate for the MeshShell fallback — the error must chain BOTH causes.
    p = petekio.PointSet.from_xyz(
        [0.0, 1.0, 2.0, 3.0, 4.0],
        [0.0, 2.0, 4.0, 6.0, 8.0],
        [-1.0, -2.0, -3.0, -4.0, -5.0],
    )
    with pytest.raises(
        ValueError,
        match=r"no regular lattice fits these points.*MeshShell fallback also failed",
    ):
        p.infer_geometry(tolerance=1e-3, max_bridge=3.5)


@pytest.mark.parametrize(
    ("x", "y", "z"),
    [
        (
            [0.0, 10.0, 2.0, 12.0, 6.0],
            [0.0, 1.0, 9.0, 11.0, 5.0],
            [-2600.0, -2610.0, -2620.0, -2630.0, -2615.0],
        ),
        (
            [0.0, 10.0, 2.0, 12.0],
            [0.0, 1.0, 9.0, 11.0],
            [-2600.0, -2610.0, -2620.0, -2630.0],
        ),
    ],
)
def test_infer_geometry_keeps_complete_small_scattered_meshes(x, y, z):
    p = petekio.PointSet.from_xyz(x, y, z)
    for kwargs in ({}, {"edge": "convex_hull"}, {"max_bridge": None}):
        with pytest.warns(UserWarning, match="MeshShell fallback"):
            shell = p.infer_geometry(tolerance=0.1, **kwargs)
        assert shell.kind == "mesh_shell"
        assert shell.n_nodes == len(x)
        assert shell.n_triangles > 0
        assert shell.components == 1
        assert len(shell.edge.rings()) == 1
        assert len(shell.labels()) == len(x)


def test_pointset_to_surface_infers_geometry_when_omitted():
    source = petekio.GridGeometry(1000.0, 2000.0, 25.0, 50.0, 6, 5, 15.0)
    x, y, z = [], [], []
    for j in range(source.nrow):
        for i in range(source.ncol):
            xi, yi = source.node_xy(i, j)
            x.append(xi)
            y.append(yi)
            z.append(100.0 + i + 2.0 * j)
    p = petekio.PointSet.from_xyz(x, y, z)

    auto = p.to_surface()
    explicit = p.to_surface(p.infer_geometry(tolerance=1e-3), "idw")

    assert auto.kind == "surface"
    assert (auto.ncol, auto.nrow) == (explicit.ncol, explicit.nrow) == (6, 5)
    ag, eg = auto.geometry, explicit.geometry
    assert (ag.xori, ag.yori, ag.xinc, ag.yinc, ag.rotation_deg) == (
        eg.xori,
        eg.yori,
        eg.xinc,
        eg.yinc,
        eg.rotation_deg,
    )
    assert auto.stats().count == explicit.stats().count
    assert auto.stats().mean == explicit.stats().mean
    assert auto.stats().min == explicit.stats().min
    assert auto.stats().max == explicit.stats().max


def test_pointset_to_surface_rejects_non_lattice_clouds_and_wrong_geom_types():
    fx, fy, fz = _faulted_blocks()
    p = petekio.PointSet.from_xyz(fx, fy, fz)

    # No geom + no regular lattice: a clear error, never an arbitrary bounding grid.
    with pytest.raises(ValueError, match="not lattice-regular"):
        p.to_surface()

    # A geometry-only fallback passed by mistake names itself and the explicit
    # value-bearing conversion to use.
    with pytest.warns(UserWarning):
        mesh = p.infer_geometry(tolerance=1e-3)
    with pytest.raises(TypeError, match="MeshShell.*to_tri_surface"):
        p.to_surface(mesh)

    # Any other wrong type names what was received.
    with pytest.raises(TypeError, match="GridGeometry"):
        p.to_surface("not a geometry")

    with pytest.raises(ValueError, match="tolerance"):
        p.to_surface(tolerance=-1.0)


def test_infer_geometry_results_carry_discoverable_kinds():
    x, y, z = [], [], []
    for j in range(3):
        for i in range(3):
            x.append(i * 10.0)
            y.append(j * 10.0)
            z.append(1.0)
    regular = petekio.PointSet.from_xyz(x, y, z)
    assert regular.kind == "point_set"
    geom = regular.infer_geometry(tolerance=1e-6)
    assert geom.kind == "grid_geometry"
    assert regular.to_surface(geom).kind == "surface"
    fx, fy, fz = _faulted_blocks()
    with pytest.warns(UserWarning):
        shell = petekio.PointSet.from_xyz(fx, fy, fz).infer_geometry(tolerance=1e-3)
    assert shell.kind == "mesh_shell"


def test_tri_surface_wireframe_edges_hide_interior_diagonals():
    x, y, z = _rotated_lattice(9, 7, 50.0, 50.0, 0.0)
    tin = petekio.PointSet.from_xyz(x, y, z).to_tri_surface()
    wf = tin.wireframe_edges()
    assert len(wf) == 9 * 6 + 7 * 8  # lattice edges only, no cell diagonals
    pts = tin.points()
    for a, b in wf:
        dx = pts[a][0] - pts[b][0]
        dy = pts[a][1] - pts[b][1]
        assert abs(math.hypot(dx, dy) - 50.0) < 1e-9


def test_tri_surface_is_deterministic():
    x, y, z = _rotated_lattice(11, 9, 50.0, 30.0, 17.0)
    p = petekio.PointSet.from_xyz(x, y, z)
    first = p.to_tri_surface()
    for _ in range(5):
        again = p.to_tri_surface()
        assert again.triangles() == first.triangles()
        assert again.points() == first.points()
        assert again.edge.rings() == first.edge.rings()


@pytest.mark.parametrize("bad", [1.0, 1.41, 2.0, 2.5])
def test_tri_surface_rejects_max_link_outside_the_band(bad):
    x, y, z = _rotated_lattice(6, 6, 50.0, 50.0, 0.0)
    with pytest.raises(ValueError, match="max_link"):
        petekio.PointSet.from_xyz(x, y, z).to_tri_surface(bad)


def test_structured_surface_round_trips_points_exactly():
    # A curvilinear, partially populated mesh with a fault-shifted node: the exact
    # shape a Petrel surface export takes. Nothing may move.
    x, y, z, col, row = [], [], [], [], []
    for j in range(5):
        for i in range(7):
            if i >= 5 and j >= 3:
                continue
            px = 1000.0 + 50.0 * i * (1.0 + 0.07 * i)
            py = 2000.0 + 50.0 * j * (1.0 + 0.05 * j)
            if i == 2 and j == 2:
                px += 9.75
                py -= 4.5
            x.append(px)
            y.append(py)
            z.append(-1800.0 - i * j)
            col.append(i + 1)
            row.append(j + 1)
    p = petekio.PointSet.from_xyz(x, y, z)
    p.column = col
    p.row = row

    back = p.to_structured_surface().to_points()

    assert len(back) == len(p)
    before = sorted(zip(col, row, x, y, z))
    bx = back.xyz()
    after = sorted(
        (int(c), int(r), q[0], q[1], q[2])
        for c, r, q in zip(back.attr("column"), back.attr("row"), bx)
    )
    assert before == after, "points -> structured surface -> points must be exact"


def test_pointset_infer_geometry_falls_back_for_curvilinear_mesh_with_topology():
    # Regular column/row, but the node spacing swells across the grid: no single
    # (xinc, yinc, rotation) lattice fits it. The strict detector must refuse to
    # invent one, and the Python convenience API must return the empty
    # StructuredShell because explicit topology is available.
    x, y, z, col, row = [], [], [], [], []
    for j in range(12):
        for i in range(12):
            x.append(1000.0 + 50.0 * i * (1.0 + 0.06 * i))
            y.append(2000.0 + 50.0 * j * (1.0 + 0.04 * j))
            z.append(10.0)
            col.append(i + 1)
            row.append(j + 1)
    p = petekio.PointSet.from_xyz(x, y, z)
    p.column = col
    p.row = row

    with pytest.warns(UserWarning, match="StructuredShell geometry"):
        inferred = p.infer_geometry(tolerance=1e-3)
    assert isinstance(inferred, petekio.StructuredShell)
    assert inferred.kind == "structured_shell"
    assert inferred.ncol == 12 and inferred.nrow == 12
    assert inferred.edge.area() > 0.0
    assert len(inferred.x()) == 12
    assert len(inferred.y()) == 12

    with pytest.raises(TypeError, match="StructuredShell.*to_structured_surface"):
        p.to_surface(inferred)

    # The exact representation still works, and reports no regular geometry.
    mesh = p.to_structured_surface(tolerance=1e-3)
    assert mesh.ncol == 12 and mesh.nrow == 12
    assert mesh.nominal_geometry is None


def test_pointset_structured_fallback_honors_edge_mode():
    # Curvilinear L-shaped topology: regular inference fails, occupied follows
    # the concavity, and convex_hull spans it. The convenience fallback must use
    # the same requested boundary as the explicit value-bearing conversion.
    x, y, z, col, row = [], [], [], [], []
    for j in range(6):
        for i in range(6):
            if i > 2 and j > 2:
                continue
            x.append(1000.0 + 40.0 * i * (1.0 + 0.055 * i) + 0.8 * i * j)
            y.append(2000.0 + 35.0 * j * (1.0 + 0.045 * j) + 0.35 * i * j)
            z.append(100.0 + i + j)
            col.append(i + 1)
            row.append(j + 1)
    points = petekio.PointSet.from_xyz(x, y, z)
    points.column = col
    points.row = row

    with pytest.warns(UserWarning, match="StructuredShell geometry"):
        inferred_occupied = points.infer_geometry(edge="occupied")
    with pytest.warns(UserWarning, match="StructuredShell geometry"):
        inferred_hull = points.infer_geometry(edge="convex_hull")
    with pytest.warns(UserWarning, match="StructuredShell geometry"):
        inferred_default = points.infer_geometry()
    explicit_occupied = points.to_structured_surface(edge="occupied").shell
    explicit_hull = points.to_structured_surface(edge="convex_hull").shell

    def canonical_rings(polygon):
        canonical = []
        for closed in polygon.rings():
            ring = [tuple(point) for point in closed[:-1]]
            variants = []
            for ordered in (ring, list(reversed(ring))):
                variants.extend(
                    tuple(ordered[offset:] + ordered[:offset])
                    for offset in range(len(ordered))
                )
            canonical.append(min(variants))
        return sorted(canonical)

    assert inferred_occupied.edge.area() < inferred_hull.edge.area()
    assert isinstance(inferred_default, petekio.StructuredShell)
    assert canonical_rings(inferred_default.edge) == canonical_rings(explicit_occupied.edge)
    assert canonical_rings(inferred_occupied.edge) == canonical_rings(explicit_occupied.edge)
    assert canonical_rings(inferred_hull.edge) == canonical_rings(explicit_hull.edge)


def test_pointset_infer_geometry_rejects_invalid_tolerance_before_fallback():
    p = petekio.PointSet.from_xyz(
        [0.0, 10.0, 0.0, 10.0],
        [0.0, 0.0, 10.0, 10.0],
        [1.0, 2.0, 3.0, 4.0],
    )
    with pytest.raises(ValueError, match="finite positive"):
        p.infer_geometry(tolerance=0.0)


def test_pointset_to_structured_surface_preserves_explicit_xy():
    p = petekio.PointSet.from_xyz(
        [0.0, 10.0, 0.0, 12.0],
        [0.0, 0.0, 10.0, 10.0],
        [100.0, 101.0, 102.0, 103.0],
    )
    p.column = [1, 2, 1, 2]
    p.row = [1, 1, 2, 2]

    s = p.to_structured_surface(edge="occupied")

    assert isinstance(s, petekio.StructuredMeshSurface)
    assert s.kind == "structured_mesh"
    assert s.ncol == 2
    assert s.nrow == 2
    assert s.node_xy(1, 1) == (12.0, 10.0)
    assert s.z(1, 1) == 103.0
    assert s.values() == [[100.0, 101.0], [102.0, 103.0]]
    assert s.stats().count == 4
    assert s.edge.area() > 0.0
    assert any("points.to_structured_surface(edge=Occupied)" in h for h in s.history())


def test_pointset_to_structured_surface_requires_topology():
    p = petekio.PointSet.from_xyz(
        [0.0, 10.0, 0.0, 10.0],
        [0.0, 0.0, 10.0, 10.0],
        [100.0, 101.0, 102.0, 103.0],
    )

    with pytest.raises(ValueError, match="requires column/row topology"):
        p.to_structured_surface()


def test_pointset_infer_geometry_falls_back_for_scattered_points():
    p = petekio.PointSet.from_xyz(
        [0.0, 11.0, 3.0, 19.0, 7.0],
        [0.0, 0.2, 8.7, 4.1, 17.3],
        [1.0, 2.0, 3.0, 4.0, 5.0],
    )
    with pytest.warns(UserWarning, match="MeshShell fallback"):
        shell = p.infer_geometry()
    assert shell.kind == "mesh_shell"
    assert shell.n_nodes == 5
    assert shell.components == 1
    assert len(shell.edge.rings()) == 1


def test_polygonset_geojson():
    poly = petekio.PolygonSet.load_geojson(SQUARE_GEOJSON)
    assert math.isclose(poly.area(), 1.0)
    assert math.isclose(poly.total_area(), 1.0)
    assert poly.contains(0.5, 0.5)
    assert not poly.contains(2.0, 2.0)
    b = poly.bbox()
    assert b.xmin == 0.0 and b.xmax == 1.0


def test_polygonset_column_math_and_assignment():
    poly = petekio.PolygonSet.from_rings([
        [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
        [[0.0, 0.0], [2.0, 0.0], [2.0, 1.0], [0.0, 1.0]],
    ])
    assert len(poly) == 2
    assert poly.area.values() == [1.0, 2.0]
    assert math.isclose(poly.area(), 3.0)
    poly.ntg = [0.5, 0.25]
    poly.net_area = poly.area * poly.ntg
    assert any("polygons.set_attr(name=net_area)" in h for h in poly.history())
    assert poly.attr("net_area") == [0.5, 0.5]
    assert poly.attr_names() == ["ntg", "net_area"]
    with pytest.raises(AttributeError):
        poly.area = [3.0, 4.0]
    with pytest.raises(TypeError):
        _ = poly + poly


def test_polygonset_clip():
    poly = petekio.PolygonSet.load_geojson(SQUARE_GEOJSON)
    g = petekio.GridGeometry(0.0, 0.0, 0.5, 0.5, 3, 3)
    s = petekio.Surface.constant(g, 5.0)
    clipped = poly.clip(s)
    # only nodes strictly inside the unit square keep their value
    assert clipped.stats().count >= 1
    assert clipped.stats().count <= s.stats().count
    assert any("mask.polygons.load_geojson" in h for h in clipped.history())
    assert any("polygons.clip(surface)" in h for h in clipped.history())


# --------------------------------------------------------------------------
# GeoData (surfaces / points / polygons)
# --------------------------------------------------------------------------


def test_geodata_surfaces():
    geo = petekio.GeoData(unit="ft")
    assert geo.unit == "ft"
    top = geo.load_surface("top", IRAP)
    assert top.ncol == 3
    assert geo.surface("top") is not None
    assert geo.surface("missing") is None
    assert len(geo.surfaces()) == 1


def test_geodata_points_polygons():
    geo = petekio.GeoData(unit="m")
    pts = geo.load_points("samples", POINTS_GEOJSON)
    assert len(pts) == 3
    # round-trip via getter (a view into the project)
    assert geo.points("samples").stats("poro").count == 3
    assert geo.points("missing") is None
    geo.load_polygons("outline", SQUARE_GEOJSON)
    assert geo.polygons("outline").contains(0.5, 0.5)
    assert geo.polygons("missing") is None


# --------------------------------------------------------------------------
# Wells: geometry, tops/logs, and the dynamic __getattr__ chain
# --------------------------------------------------------------------------


def _geo_with_well():
    geo = petekio.GeoData(unit="m")
    geo.load_well("15/9-A1", (1200.0, 1500.0), 82.0, WELL_DIR)
    return geo


def test_well_geometry():
    geo = _geo_with_well()
    w = geo.well("15/9-A1")
    assert w.id == "15/9-A1"
    assert w.head == (1200.0, 1500.0)
    assert w.kb == 82.0
    # synthesized vertical trajectory: xyz.z = negative-down elevation (kb - md),
    # tvd = md - kb (positive-down).
    x, y, z = w.xyz(2420.0)
    assert math.isclose(x, 1200.0)
    assert math.isclose(z, 82.0 - 2420.0)
    assert math.isclose(w.tvd(2420.0), 2420.0 - 82.0)
    assert geo.well("missing") is None


def test_well_top_log_explicit():
    geo = _geo_with_well()
    w = geo.well("15/9-A1")
    brent = w.top("Brent")
    assert brent is not None
    assert math.isclose(brent.top_md, 2400.0)
    assert math.isclose(brent.base_md, 2450.0)
    assert math.isclose(brent.thickness_md(), 50.0)
    lv = brent.log("NTG")
    assert lv is not None
    st = lv.stats()
    assert st.count == 5
    assert math.isclose(st.mean, 0.3, rel_tol=1e-9)
    assert len(lv.md()) == 5
    assert lv.values()[0] == 0.1
    assert w.top("Nope") is None


def test_well_getattr_chain():
    geo = _geo_with_well()
    w = geo.well("15/9-A1")
    # w.brent -> Interval; w.brent.ntg -> Stats
    ntg = w.brent.ntg
    assert isinstance(ntg, petekio.Stats)
    assert ntg.count == 5
    assert math.isclose(ntg.mean, 0.3, rel_tol=1e-9)
    # w.brent.ntg.mean (chained attribute)
    assert math.isclose(w.brent.ntg.mean, 0.3, rel_tol=1e-9)
    # unknown top -> AttributeError
    with pytest.raises(AttributeError):
        _ = w.nonexistent_top
    # unknown log on a real interval -> AttributeError
    with pytest.raises(AttributeError):
        _ = w.brent.no_such_log


def test_well_full_log_view():
    geo = _geo_with_well()
    w = geo.well("15/9-A1")
    gr = w.log("GR")
    assert gr is not None
    # GR has a NULL sample -> stats skip it
    assert gr.stats().count == 4
    assert w.log("missing") is None


def test_wells_broadcast():
    geo = petekio.GeoData(unit="m")
    geo.load_well("15/9-A1", (1200.0, 1500.0), 82.0, WELL_DIR)
    geo.load_well("only-logs", (0.0, 0.0), 0.0, LAS)
    wells = geo.wells
    assert len(wells) == 2
    # filter by a Python predicate over Well
    east = wells.filter(lambda w: w.head[0] > 1000.0)
    assert east.len() == 1
    assert east.iter()[0].id == "15/9-A1"
    # tops() narrows to wells carrying the marker
    brent = wells.tops("Brent")
    assert brent.len() == 1
    # broadcast a log over the narrowed view -> list[Stats]
    means = [s.mean for s in brent.ntg]
    assert len(means) == 1
    assert math.isclose(means[0], 0.3, rel_tol=1e-9)
    # attribute-style top selection then log broadcast
    stats_list = geo.wells.brent.ntg
    assert len(stats_list) == 1
    assert math.isclose(stats_list[0].mean, 0.3, rel_tol=1e-9)


def test_trajectory_from_stations_min_curvature():
    # Worked survey: vertical hold, then build to 57/80/89 deg.
    survey = [
        (0, 0, 145), (1200, 0, 145), (1900, 57, 145), (2200, 57, 145),
        (2500, 80, 135), (3700, 80, 135), (3900, 89, 135), (4400, 89, 135),
    ]
    kb = 27.3
    t = petekio.Trajectory.from_stations(survey, head=(1000.0, 2000.0), kb=kb)
    assert t.md_range() == (0.0, 4400.0)
    # RKB TVD (= TVDSS + kb) reproduces a hand-checked reference at the stations.
    for md, tvd_rkb in [(1200, 1200.0), (1900, 1790.116), (2500, 2062.961)]:
        assert math.isclose(t.tvd(md) + kb, tvd_rkb, abs_tol=0.05)
    # Vertical section: tvd = md - kb.
    assert math.isclose(t.tvd(600.0), 600.0 - kb, abs_tol=1e-9)
    # Position tuple (x = head.x + easting offset) + out-of-range.
    x, y, z = t.xyz(1900.0)
    assert math.isclose(x, 1000.0 + 183.778, abs_tol=0.5)
    assert t.tvd(5000.0) is None


def test_sidetrack_zones_and_stats(tmp_path):
    # Single-bore well (no .wellpath) → main bore ""; GR/NTG logs + Brent/Dunlin tops.
    geo = petekio.GeoData(unit="m")
    geo.load_well("15/9-A1", head=(0.0, 0.0), kb=0.0, files=WELL_DIR)
    w = geo.well("15/9-A1")
    assert w.bores() == [""]
    st = w.sidetrack("")
    assert st is not None
    assert "NTG" in st.mnemonics()
    # whole-bore stats
    assert st.log_stats("NTG").count > 0
    # per-zone stats: Brent zone exists with an NTG mean
    zs = dict(st.zone_stats("NTG"))
    assert "Brent" in zs and zs["Brent"].count > 0
    assert any(name == "Brent" for name, _, _ in st.zones())

    # Petrel well-tops routing via load_well_tops (synthetic file in tmp).
    tops = tmp_path / "wt.tops"
    tops.write_text(
        "# Petrel well tops\nVERSION 2\nBEGIN HEADER\nX\nY\nZ\nTWT\nTWT2\nage\nMD\nPVD\nType\nSurface\nWell\nEND HEADER\n"
        '1 2 -3 -999 -999 -999 2425.0 -3 Horizon "Synthetic top" "15/9-A1"\n'
        '1 2 -3 -999 -999 -999 2440.0 -3 Other "OWC" "15/9-A1"\n'
    )
    added = geo.load_well_tops(str(tops))
    assert added == 1  # Horizon counted as a zone top.
    assert w.contact("owc") == ("OWC", 2440.0)
    assert st.contact("OWC") == ("OWC", 2440.0)
    assert st.contacts() == [("OWC", 2440.0)]


def test_load_well_optional_head_kb_from_wellpath(tmp_path):
    # A .wellpath supplies head/kb, so they can be omitted from load_well.
    wd = tmp_path / "W"
    wd.mkdir()
    (wd / "99_9-1_A.wellpath").write_text(
        "# WELL TRACE FROM PETREL\n"
        "# WELL HEAD X-COORDINATE: 1234.0 (m)\n"
        "# WELL HEAD Y-COORDINATE: 5678.0 (m)\n"
        "# WELL DATUM (KB, Kelly bushing, from MSL): 30.0 (m)\n"
        "# CRS: ED50 / UTM zone 31N\n=====\n"
        "MD X Y Z TVD DX DY AZIM_TN INCL DLS AZIM_GN\n"
        "0 1234.0 5678.0 0 0 0 0 145 0 0 145\n"
        "1000 1234.0 5678.0 -1000 1000 0 0 145 0 0 145\n"
    )
    geo = petekio.GeoData(unit="m")
    geo.load_well("99/9-1", files=str(wd))          # head/kb omitted
    w = geo.well("99/9-1")
    assert w.head == (1234.0, 5678.0)
    assert math.isclose(w.kb, 30.0)
    # A single .wellpath is the well's one bore — the main bore "" — so the well
    # positions through it directly (the single-trajectory routing rule).
    assert w.bores() == [""]
    assert w.xyz(500.0) is not None  # positioned through the sole trajectory
    # `files` is still required
    with pytest.raises(ValueError):
        geo.load_well("x")


def _wp_body(rows):
    return (
        "# WELL TRACE FROM PETREL\n"
        "# WELL HEAD X-COORDINATE: 1000.0 (m)\n"
        "# WELL HEAD Y-COORDINATE: 2000.0 (m)\n"
        "# WELL DATUM (KB, Kelly bushing, from MSL): 27.3 (m)\n"
        "# CRS: ED50 / UTM zone 31N\n=====\n"
        "MD X Y Z TVD DX DY AZIM_TN INCL DLS AZIM_GN\n" + rows
    )


def test_multibore_well_selection(tmp_path):
    # R-a: a multi-sidetrack well (two .wellpath bores A + ST2) must not silently
    # resolve the top-level accessors through the empty main bore.
    wd = tmp_path / "99_9-1"
    wd.mkdir()
    (wd / "99_9-1_A.wellpath").write_text(
        _wp_body(
            "0 1000 2000 0 0 0 0 145 0 0 145\n"
            "2000 1000 2000 -2000 2000 0 0 145 0 0 145\n"
        )
    )
    (wd / "99_9-1_ST2.wellpath").write_text(
        _wp_body(
            "0 1000 2000 0 0 0 0 145 0 0 145\n"
            "1800 1050 1970 -1790 1795 50 -30 145 10 1 145\n"
        )
    )
    geo = petekio.GeoData(unit="m")
    geo.load_well("99/9-1", files=str(wd))
    w = geo.well("99/9-1")

    # Bores enumerable; multi-bore flagged; no default selected yet.
    assert set(w.bores()) == {"", "A", "ST2"}
    assert w.is_multibore is True
    assert w.default_bore is None

    # Top-level (bore-picking) access raises — no silent empty — and names the bores.
    with pytest.raises(ValueError, match="bores"):
        w.xyz(1000.0)
    with pytest.raises(ValueError, match="ST2"):
        w.tvd(1000.0)
    with pytest.raises(ValueError):
        w.log("GR")

    # Per-bore access is first-class regardless.
    a = w.sidetrack("A")
    assert a.tvd(1200.0) is not None and a.md_range() == (0.0, 2000.0)

    # Selecting a default bore routes the top-level accessors through it.
    with pytest.raises(ValueError):
        w.set_default_bore("Z")  # no such bore → loud
    w.set_default_bore("A")
    assert w.default_bore == "A"
    assert w.xyz(1200.0) is not None
    assert math.isclose(w.tvd(1200.0), 1200.0 - 27.3)


def test_zone_stats_single_zone():
    # zone_stats(mnemonic, zone) returns one Stats (or None) — no dict() needed.
    geo = petekio.GeoData(unit="m")
    geo.load_well("15/9-A1", head=(0.0, 0.0), kb=0.0, files=WELL_DIR)
    st = geo.well("15/9-A1").sidetrack("")
    full = dict(st.zone_stats("NTG"))
    assert "Brent" in full
    one = st.zone_stats("NTG", "Brent")
    assert one is not None and math.isclose(one.mean, full["Brent"].mean)
    assert st.zone_stats("NTG", "brent") is not None  # case-insensitive
    assert st.zone_stats("NTG", "Nope") is None  # absent zone → None
    assert isinstance(st.zone_stats("NTG"), list)  # no zone arg → list (compat)


def test_strat_order_global_column(tmp_path):
    # The lithostratigraphic column merges across every well in the tops file:
    # FIELD-3 develops Sand above Mid, FIELD-2 develops Lower below Mid, so a
    # Sand listed last in the file still sorts to its true depth.
    geo = petekio.GeoData(unit="m")
    assert geo.strat_order == []  # empty before any tops are loaded
    tops = tmp_path / "field.tops"
    tops.write_text(
        "# Petrel well tops\nVERSION 2\nBEGIN HEADER\nX\nY\nZ\nTWT\nTWT2\nage\nMD\nPVD\nType\nSurface\nWell\nEND HEADER\n"
        '1 2 -1 -999 -999 -999 100.0 -1 Horizon "Top" "FIELD-1"\n'
        '1 2 -1 -999 -999 -999 120.0 -1 Horizon "Mid" "FIELD-1"\n'
        '1 2 -1 -999 -999 -999 120.0 -1 Horizon "Mid" "FIELD-2"\n'
        '1 2 -1 -999 -999 -999 130.0 -1 Horizon "Lower" "FIELD-2"\n'
        '1 2 -1 -999 -999 -999 110.0 -1 Horizon "Sand" "FIELD-3"\n'
        '1 2 -1 -999 -999 -999 120.0 -1 Horizon "Mid" "FIELD-3"\n'
        '1 2 -1 -999 -999 -999 120.0 -1 Horizon "Sand" "FIELD-1"\n'
    )
    geo.load_well_tops(str(tops))
    assert geo.strat_order == ["Top", "Sand", "Mid", "Lower"]


def test_strat_hint(tmp_path):
    header = "# Petrel well tops\nVERSION 2\nBEGIN HEADER\nX\nY\nZ\nTWT\nTWT2\nage\nMD\nPVD\nType\nSurface\nWell\nEND HEADER\n"
    body = (
        '1 2 -1 -999 -999 -999 100.0 -1 Horizon "Top" "W1"\n'
        '1 2 -1 -999 -999 -999 120.0 -1 Horizon "Alpha top" "W1"\n'
        '1 2 -1 -999 -999 -999 120.0 -1 Horizon "Beta top" "W1"\n'  # coincident → stalemate
        '1 2 -1 -999 -999 -999 200.0 -1 Horizon "Deep" "W1"\n'
    )
    tops = tmp_path / "w.tops"
    tops.write_text(header + body)

    # Default tiebreak: Alpha before Beta.
    g = petekio.GeoData(unit="m")
    g.load_well_tops(str(tops))
    assert g.strat_order == ["Top", "Alpha top", "Beta top", "Deep"]

    # Shorthand + partial names: "Beta above Alpha".
    g = petekio.GeoData(unit="m")
    g.strat_hint("Beta < Alpha")
    g.load_well_tops(str(tops))
    assert g.strat_order == ["Top", "Beta top", "Alpha top", "Deep"]

    # Explicit kwargs form is equivalent.
    g = petekio.GeoData(unit="m")
    g.strat_hint(above="Beta top", below="Alpha top")
    g.load_well_tops(str(tops))
    assert g.strat_order == ["Top", "Beta top", "Alpha top", "Deep"]

    # Data wins: a hint contradicting a strict MD relationship is ignored.
    g = petekio.GeoData(unit="m")
    g.strat_hint("Deep < Top")  # "Deep above Top" — but data has Top above Deep
    g.load_well_tops(str(tops))
    assert g.strat_order.index("Top") < g.strat_order.index("Deep")

    # Errors: no operator, and mixing both forms.
    g = petekio.GeoData(unit="m")
    with pytest.raises(ValueError):
        g.strat_hint("no operator")
    with pytest.raises(ValueError):
        g.strat_hint("Beta < Alpha", above="x", below="y")


def test_zone_table():
    pd = pytest.importorskip("pandas")
    geo = petekio.GeoData(unit="m")
    geo.load_well("15/9-A1", head=(0.0, 0.0), kb=0.0, files=WELL_DIR)
    w = geo.well("15/9-A1")

    t = w.zone_table("NTG", stats=["mean", "p50"])
    assert list(t.columns) == ["zone", "bore", "mean", "p50"]
    assert str(t["zone"].dtype) == "category" and t["zone"].cat.ordered
    assert (t["mean"] == 0).sum() == 0  # drop-empty default: no zero-count rows
    # pivots cleanly (the boilerplate the request removes)
    piv = t.pivot(index="zone", columns="bore", values="mean")
    assert "Brent" in list(piv.index)

    # default stat is mean
    assert list(w.zone_table("NTG").columns) == ["zone", "bore", "mean"]
    # unknown stat → ValueError
    with pytest.raises(ValueError):
        w.zone_table("NTG", stats=["bogus"])

    # WellsView level: bore identifies well + sidetrack
    tv = geo.wells.zone_table("NTG")
    assert set(["zone", "bore", "mean"]).issubset(tv.columns)
    assert all(b == "15/9-A1" or b.startswith("15/9-A1 ") for b in tv["bore"].unique())

    # pivot=True → zone is the index, bore the columns (single stat → flat)
    p = w.zone_table("NTG", pivot=True)
    assert p.index.name == "zone" and "Brent" in list(p.index)
    assert "bore" not in p.columns  # bore became the column axis
    # several stats → MultiIndex (stat, bore)
    pm = w.zone_table("NTG", stats=["mean", "p50"], pivot=True)
    assert pm.index.name == "zone" and pm.columns.nlevels == 2
    assert {s for s, _ in pm.columns} == {"mean", "p50"}  # top level = the stats

    # decimals=N rounds the stat values
    d = w.zone_table("NTG", decimals=2)
    assert (d["mean"].dropna().round(2) == d["mean"].dropna()).all()

    # aggregate=True → grouped (zone, bore); pooled "all" row first per zone.
    # (weighted=False so sum stays Σvalue and the pooled-mean identity holds;
    #  thickness-weighting is exercised in test_zone_table_thickness_weighting.)
    a = w.zone_table("NTG", stats=["mean", "sum", "count"], aggregate=True, weighted=False)
    assert list(a.index.names) == ["zone", "bore"]
    brent = a.xs("Brent", level="zone")
    assert "all" in brent.index
    # pooled mean is sample-weighted: Σsum / Σcount over the per-bore rows
    bores_only = brent.drop("all")
    pooled = bores_only["sum"].sum() / bores_only["count"].sum()
    assert math.isclose(brent.loc["all", "mean"], pooled, rel_tol=1e-9)
    # pivot and aggregate are mutually exclusive
    with pytest.raises(ValueError):
        w.zone_table("NTG", pivot=True, aggregate=True)

    # zones= keeps only the requested zones (case-insensitive), order preserved
    one = w.zone_table("NTG", zones=["Brent"])
    assert set(one["zone"]) == {"Brent"}
    assert set(w.zone_table("NTG", zones=["brent"])["zone"]) == {"Brent"}  # case-insensitive
    assert w.zone_table("NTG", zones=["does not exist"]).empty  # unknown → no rows
    # composes with aggregate
    agg = w.zone_table("NTG", zones=["Brent"], aggregate=True)
    assert set(agg.index.get_level_values("zone")) == {"Brent"}


def test_zone_table_thickness_weighting(tmp_path):
    pytest.importorskip("pandas")
    # One log, irregular spacing in zone A: three dense low-phi samples then one
    # sparse high-phi sample. Plain mean = 0.15; thickness-weighted lifts it
    # because the sparse 0.30 sample represents a much thicker interval.
    wd = tmp_path / "TESTW"
    wd.mkdir()
    (wd / "TESTW.las").write_text(
        "~Version\n VERS. 2.0 :\n WRAP. NO :\n~Well\n STRT.M 1000 :\n STOP.M 1010 :\n"
        " STEP.M 1 :\n NULL. -999.25 :\n~Curve\n DEPT.M :\n PHIE.m3/m3 :\n~ASCII\n"
        "1000 0.10\n1001 0.10\n1002 0.10\n1010 0.30\n"
    )
    tops = tmp_path / "t.tops"
    tops.write_text(
        "# Petrel well tops\nVERSION 2\nBEGIN HEADER\nX\nY\nZ\nTWT\nTWT2\nage\nMD\nPVD\nType\nSurface\nWell\nEND HEADER\n"
        '1 2 -1 -999 -999 -999 1000.0 -1 Horizon "A" "TESTW"\n'
        '1 2 -1 -999 -999 -999 1015.0 -1 Horizon "B" "TESTW"\n'
    )
    geo = petekio.GeoData(unit="m")
    geo.load_well("TESTW", files=str(wd))
    geo.load_well_tops(str(tops))
    w = geo.well("TESTW")

    def amean(weighted):
        return w.zone_table("PHIE", aggregate=True, weighted=weighted).xs("A", level="zone").loc[
            "all", "mean"
        ]

    assert math.isclose(amean(False), 0.15, rel_tol=1e-9)  # plain mean of [.1,.1,.1,.3]
    # dz (midpoint) = [1, 1, 4.5, 8] → (.1+.1+.45+2.4)/14.5
    assert math.isclose(amean(True), 3.05 / 14.5, rel_tol=1e-6)
    assert amean(True) > amean(False)  # sparse high-phi sample no longer under-weighted

    # samples / gross stats
    t = w.zone_table("PHIE", stats=["samples", "gross"], aggregate=True)
    a = t.xs("A", level="zone").loc["all"]
    assert a["samples"] == 4 and a["gross"] > 0


def test_project_persistence(tmp_path):
    geo = petekio.GeoData(unit="m")
    geo.load_well("15/9-A1", head=(0.0, 0.0), kb=0.0, files=WELL_DIR)
    geo.set_owner("kk")
    geo.set_tags(["demo"])
    geo.set_element_tags("15/9-A1", ["keep"])
    geo.put_model_section("model/seg/props", ["keep"], 3, b"\x00\xffmodel")
    geo.put_model_section("model/other", ["drop"], 1, b"\x09")
    src = str(tmp_path / "p.pproj")
    geo.save(src)

    # inspect: manifest only
    info = petekio.GeoData.inspect(src)
    assert info["owner"] == "kk" and "demo" in info["tags"] and info["unit"] == "Metres"
    assert any(k == "well" and n == "15/9-A1" for k, n in info["elements"])

    # open: full round-trip incl opaque model bytes
    re = petekio.GeoData.open(src)
    assert re.owner == "kk" and re.tags == ["demo"]
    assert re.well("15/9-A1") is not None
    assert re.model_section("model/seg/props") == (3, b"\x00\xffmodel")
    assert set(re.model_section_names()) == {"model/seg/props", "model/other"}

    # export by tag → shareable subset
    sub = str(tmp_path / "sub.pproj")
    petekio.GeoData.export(src, sub, ["keep"])
    s = petekio.GeoData.open(sub)
    assert s.model_section_names() == ["model/seg/props"]
    assert s.well("15/9-A1") is not None

    # split then merge
    wonly = str(tmp_path / "w.pproj")
    petekio.GeoData.split(src, wonly, ["15/9-A1"])
    merged = str(tmp_path / "m.pproj")
    petekio.GeoData.merge(wonly, sub, merged)
    m = petekio.GeoData.open(merged)
    assert m.well("15/9-A1") is not None
    assert m.model_section("model/seg/props") == (3, b"\x00\xffmodel")
def test_format_detector_is_public(tmp_path):
    p = tmp_path / "misnamed.csv"
    p.write_text("~Version\n VERS. 2.0 :\n~Well\n STRT.M 100 :\n")

    assert "detect" in petekio.__all__
    assert "FormatKind" in petekio.__all__
    assert petekio.detect(str(p)) == petekio.FormatKind.Las
