"""End-to-end tests for the petekio PyO3 bindings, exercised against the
committed fixtures under ``tests/fixtures/``. Run with ``pytest`` against an
installed wheel.
"""

import math
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
    assert base.minus(top).stats().mean == 30.0
    # thickness staticmethod (base - top)
    assert petekio.Surface.thickness(top, base).stats().mean == 30.0
    # geometry mismatch raises
    other = petekio.Surface.constant(petekio.GridGeometry(0.0, 0.0, 10.0, 10.0, 4, 4), 1.0)
    with pytest.raises(ValueError):
        _ = top + other


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


def test_surface_resample_identity():
    s = petekio.Surface.load_irap_classic(IRAP)
    r = s.resample(s.geometry)
    assert r.ncol == s.ncol and r.nrow == s.nrow


def test_surface_attr_access():
    g = petekio.GridGeometry(0.0, 0.0, 10.0, 10.0, 2, 2)
    s = petekio.Surface.constant(g, 1.0)
    seismic = petekio.Surface.constant(g, 7.0)
    s.set_attr("seismic", seismic)
    assert "seismic" in s.attr
    assert s.attr_names() == ["seismic"]
    # indexed access promotes to a Surface
    promoted = s.attr["seismic"]
    assert promoted.stats().mean == 7.0
    # ln() on the promoted attribute works (returns a Surface, not ndarray)
    assert math.isclose(s.attr["seismic"].ln().stats().mean, math.log(7.0))
    # call form too
    assert s.attr("seismic").stats().mean == 7.0
    with pytest.raises(KeyError):
        _ = s.attr["missing"]


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


def test_pointset_to_surface():
    p = petekio.PointSet.load_geojson(POINTS_GEOJSON)
    g = petekio.GridGeometry(0.0, 0.0, 5.0, 5.0, 3, 3)
    surf = p.to_surface(g, "idw")
    assert surf.ncol == 3 and surf.nrow == 3
    # nearest method also valid
    surf2 = p.to_surface(g, "nearest")
    assert surf2.stats().count > 0
    with pytest.raises(TypeError):
        p.to_surface(g, "bogus")


def test_polygonset_geojson():
    poly = petekio.PolygonSet.load_geojson(SQUARE_GEOJSON)
    assert math.isclose(poly.area(), 1.0)
    assert poly.contains(0.5, 0.5)
    assert not poly.contains(2.0, 2.0)
    b = poly.bbox()
    assert b.xmin == 0.0 and b.xmax == 1.0


def test_polygonset_clip():
    poly = petekio.PolygonSet.load_geojson(SQUARE_GEOJSON)
    g = petekio.GridGeometry(0.0, 0.0, 0.5, 0.5, 3, 3)
    s = petekio.Surface.constant(g, 5.0)
    clipped = poly.clip(s)
    # only nodes strictly inside the unit square keep their value
    assert clipped.stats().count >= 1
    assert clipped.stats().count <= s.stats().count


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
