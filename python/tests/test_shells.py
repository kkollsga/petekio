"""The three-level geometry-shell system: iso-lines, value layers,
multi-attribute lanes, conversions, and backward compatibility.

Synthetic data only.
"""

import math

import petekio
import pytest


def plane_surface():
    """An 11 x 5 grid, 10 m spacing, z = 2x + 100 (a tilted plane)."""
    geom = petekio.GridGeometry(0.0, 0.0, 10.0, 10.0, 11, 5)
    xs, ys, zs = [], [], []
    for j in range(5):
        for i in range(11):
            xs.append(10.0 * i)
            ys.append(10.0 * j)
            zs.append(2.0 * (10.0 * i) + 100.0)
    pts = petekio.PointSet.from_xyz(xs, ys, zs)
    return pts.to_surface(geom, "nearest")


def lattice_points(ncol=9, nrow=7, xinc=50.0, yinc=50.0):
    xs, ys, zs = [], [], []
    for j in range(nrow):
        for i in range(ncol):
            xs.append(1000.0 + xinc * i)
            ys.append(2000.0 + yinc * j)
            zs.append(-1800.0 - i - j)
    return petekio.PointSet.from_xyz(xs, ys, zs)


# ---- iso-lines: the seam contract -------------------------------------------


def test_iso_lines_contract_and_analytic_positions():
    s = plane_surface()
    out = s.iso_lines(interval=50.0)
    assert isinstance(out, list)
    levels = [lv for lv, _ in out]
    assert levels == [100.0, 150.0, 200.0, 250.0, 300.0]
    for lv, lines in out:
        assert isinstance(lines, list)
        if lv <= 100.0:
            continue  # the exact minimum has no crossing
        assert len(lines) == 1, f"one straight line per level {lv}"
        expect_x = (lv - 100.0) / 2.0
        for p in lines[0]:
            assert isinstance(p, tuple) and len(p) == 2
            assert abs(p[0] - expect_x) < 1e-9  # exactly computable x


def test_iso_lines_explicit_levels_win_and_args_are_validated():
    s = plane_surface()
    out = s.iso_lines(interval=50.0, levels=[137.0, 253.0])
    assert [lv for lv, _ in out] == [137.0, 253.0]
    try:
        s.iso_lines()
        raise AssertionError("iso_lines() without interval/levels must raise")
    except ValueError:
        pass


def test_iso_lines_on_all_three_levels_agree():
    s = plane_surface()
    sm = s.to_structured_mesh()
    tri = s.to_tri_surface()
    for surf in (s, sm, tri):
        out = surf.iso_lines(levels=[200.0])
        (lv, lines) = out[0]
        assert lv == 200.0
        assert len(lines) == 1
        for p in lines[0]:
            assert abs(p[0] - 50.0) < 1e-9


# ---- value layer: the viewer trimesh dict (do not change) -------------------


def test_value_layer_dict_shape():
    s = plane_surface()
    layer = s.value_layer()
    assert set(layer.keys()) == {"kind", "name", "nodes", "triangles", "values", "range"}
    assert layer["kind"] == "trimesh"
    assert layer["name"] == "values"
    assert len(layer["nodes"]) == 11 * 5
    assert all(len(n) == 2 for n in layer["nodes"])
    assert len(layer["triangles"]) == 2 * 10 * 4
    assert all(len(t) == 3 for t in layer["triangles"])
    assert len(layer["values"]) == len(layer["nodes"])
    assert layer["range"] == [100.0, 300.0]


def test_value_layer_attr_lane_and_all_levels():
    s = plane_surface()
    s.set_attr("amp", s * 0.5)
    for surf in (s, s.to_structured_mesh(), s.to_tri_surface()):
        layer = surf.value_layer(attr="amp")
        assert layer["name"] == "amp"
        assert layer["range"] == [50.0, 150.0]


# ---- multi-attribute lanes + immutability -----------------------------------


def test_attribute_lanes_carry_upward_one_to_one():
    s = plane_surface()
    s.set_attr("amp", s * 0.5)

    sm = s.to_structured_mesh()
    assert sm.kind == "structured_mesh"
    assert sm.attr_names() == ["amp"]

    for tri in (s.to_tri_surface(), sm.to_tri_surface()):
        assert tri.attr_names() == ["amp"]
        amp = tri.attr("amp")  # promoted on the same shell
        pts = tri.points()
        vals = amp.values()
        assert len(vals) == len(pts)
        for (x, y, z), a in zip(pts, vals):
            assert abs(z - (2.0 * x + 100.0)) < 1e-9
            assert abs(a - 0.5 * z) < 1e-9


def test_set_attr_returns_a_new_object():
    tri = plane_surface().to_tri_surface()
    n = tri.n_points
    tri2 = tri.set_attr("flag", [1.0] * n)
    assert tri2.attr_names() == ["flag"]
    assert tri.attr_names() == []  # the original is untouched

    sm = plane_surface().to_structured_mesh()
    rows = sm.values()
    sm2 = sm.set_attr("copy", rows)
    assert sm2.attr_names() == ["copy"]
    assert sm.attr_names() == []
    assert sm2.attr("copy").values() == rows


# ---- shells ------------------------------------------------------------------


def test_shell_accessors():
    s = plane_surface()
    sm = s.to_structured_mesh()
    shell2 = sm.shell
    assert (shell2.ncol, shell2.nrow) == (11, 5)
    assert shell2.node_xy(3, 2) == (30.0, 20.0)
    assert shell2.nominal_geometry is not None

    tri = s.to_tri_surface()
    shell3 = tri.shell
    assert shell3.n_nodes == tri.n_points
    assert shell3.n_triangles == tri.n_triangles
    assert shell3.components == 1
    labels = shell3.labels()
    assert len(labels) == shell3.n_nodes
    assert all(lab is not None for lab in labels)
    # nodes are 2-D (x, y) — the shell is never a function of z
    assert all(len(n) == 2 for n in shell3.nodes())
    # level 2 shell explodes into the level 3 shell
    assert shell2.to_mesh_shell().n_nodes == shell3.n_nodes


def test_infer_grid_round_trips_and_refuses_irregular():
    s = plane_surface()
    for g in (
        s.to_structured_mesh().infer_grid(),
        s.to_tri_surface().infer_grid(),
        s.to_tri_surface().shell.infer_grid(),
    ):
        assert (g.ncol, g.nrow) == (11, 5)
        assert abs(g.xinc - 10.0) < 1e-9
        assert abs(g.yinc - 10.0) < 1e-9


def test_downward_resample_carries_attributes():
    s = plane_surface()
    s.set_attr("amp", s * 0.5)
    target = petekio.GridGeometry(10.0, 10.0, 10.0, 10.0, 3, 3)
    for down in (
        s.to_structured_mesh().resample(target, "nearest"),
        s.to_tri_surface().resample(target, "nearest"),
    ):
        assert isinstance(down, petekio.Surface)
        assert down.attr_names() == ["amp"]
        v = down.sample(20.0, 20.0)
        assert abs(v - (2.0 * 20.0 + 100.0)) < 1e-9


# ---- backward compatibility (the Python surface API is a contract) -----------


def test_tri_surface_legacy_methods_unchanged():
    tin = lattice_points().to_tri_surface()
    assert tin.kind == "tri_surface"
    assert tin.n_points == 9 * 7
    assert tin.n_triangles == 2 * 8 * 6
    assert tin.components == 1
    pts = tin.points()
    assert isinstance(pts[0], tuple) and len(pts[0]) == 3
    assert tin.xyz() == pts
    assert len(tin.triangles()) == tin.n_triangles
    assert len(tin.wireframe_edges()) == 9 * 6 + 7 * 8  # quad-dominant
    assert tin.edge is not None
    bb = tin.bbox()
    assert bb.xmin == 1000.0 and bb.ymin == 2000.0
    st = tin.stats()
    assert st.count == tin.n_points
    assert len(tin.to_points()) == tin.n_points
    assert isinstance(tin.history(), list)


def test_infer_geometry_still_returns_grid_or_tri_surface():
    # A regular lattice → GridGeometry.
    p = lattice_points()
    g = p.infer_geometry(tolerance=1e-3)
    assert isinstance(g, petekio.GridGeometry)

    # A fault-cut cloud → TriSurface fallback, exactly as before.
    xs, ys, zs = [], [], []
    for j in range(9):
        for i in range(6):
            xs.append(50.0 * i)
            ys.append(50.0 * j)
            zs.append(-1800.0)
    for j in range(9):
        for i in range(8, 14):
            xs.append(50.0 * i + 20.0)
            ys.append(50.0 * j + 25.0)
            zs.append(-1900.0)
    p = petekio.PointSet.from_xyz(xs, ys, zs)
    with pytest.warns(UserWarning, match="TriSurface fallback"):
        t = p.infer_geometry(tolerance=1e-3)
    assert isinstance(t, petekio.TriSurface)
    assert t.components == 2  # the fault is honoured, not bridged


def test_structured_mesh_legacy_methods_unchanged():
    pts = lattice_points(7, 5)
    labelled, report = pts.detect_topology()
    assert report.verified
    sm = labelled.to_structured_surface(1e-3, "occupied")
    assert sm.kind == "structured_mesh"
    assert (sm.ncol, sm.nrow) == (7, 5)
    assert len(sm.values()) == 5 and len(sm.values()[0]) == 7
    assert sm.node_xy(0, 0) == (1000.0, 2000.0)
    assert not math.isnan(sm.z(1, 1))
    assert len(sm.to_points()) == 7 * 5
    assert sm.stats().count == 7 * 5
    assert sm.edge is not None
    assert sm.bbox().xmin == 1000.0
