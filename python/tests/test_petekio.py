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
    # synthesized vertical trajectory: tvd = md - kb
    x, y, z = w.xyz(2420.0)
    assert math.isclose(x, 1200.0)
    assert math.isclose(z, 2420.0 - 82.0)
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
    assert added == 1  # Horizon kept, Other (OWC) skipped


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
    assert len(w.bores()) == 2  # main "" + the one wellpath bore
    # `files` is still required
    with pytest.raises(ValueError):
        geo.load_well("x")


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
