"""Task 3 — Python petrophysics access: Sidetrack.log (W3), net-conditioned
zone aggregation + geomean (W4), PointSet.z_stats (W10), and in-memory
PointSet/PolygonSet constructors. Fixtures are hand-authored to spec in tmp."""
import math
import petekio


def _well_dir(tmp_path):
    las = (
        "~Version\n VERS. 2.0 :\n WRAP. NO :\n"
        "~Well\n STRT.M 100.0 :\n STOP.M 130.0 :\n STEP.M 10.0 :\n NULL. -999.25 :\n"
        "~Curve\n DEPTH.M :\n PHIE.v/v :\n SW.v/v :\n PERM.mD :\n"
        "~ASCII\n"
        "100.0 0.20 0.30 100.0\n"   # net
        "110.0 0.05 0.30   5.0\n"   # phi < cutoff -> not net
        "120.0 0.20 0.60  50.0\n"   # sw > cutoff  -> not net
        "130.0 0.25 0.20 400.0\n"   # net
    )
    (tmp_path / "99_9-1.las").write_text(las)
    (tmp_path / "99_9-1.csv").write_text("name,md\nSand,100\nBase,140\n")
    return str(tmp_path)


def test_sidetrack_log_per_sample(tmp_path):
    geo = petekio.GeoData(unit="m")
    geo.load_well("99/9-1", head=(0.0, 0.0), kb=0.0, files=_well_dir(tmp_path))
    st = geo.well("99/9-1").sidetrack("")
    lv = st.log("PHIE")
    assert lv is not None
    assert lv.md() == [100.0, 110.0, 120.0, 130.0]
    assert lv.values() == [0.20, 0.05, 0.20, 0.25]
    assert st.log("NOPE") is None
    # geomean over a view (permeability)
    g = st.log("PERM").geomean()
    assert math.isclose(g, (100.0 * 5.0 * 50.0 * 400.0) ** 0.25, rel_tol=1e-9)


def test_net_conditioned_zone_aggregation(tmp_path):
    geo = petekio.GeoData(unit="m")
    geo.load_well("99/9-1", head=(0.0, 0.0), kb=0.0, files=_well_dir(tmp_path))
    st = geo.well("99/9-1").sidetrack("")
    # net PHIE (arithmetic) over Sand: samples at 100 & 130 pass -> mean 0.225.
    phi = dict(st.net_zone_stats("PHIE", phi="PHIE", sw="SW"))
    assert math.isclose(phi["Sand"].mean, 0.225, rel_tol=1e-9)
    assert phi["Sand"].count == 2
    # net PERM geometric mean over Sand: geomean(100, 400) = 200.
    perm = dict(st.net_zone_stats("PERM", phi="PHIE", sw="SW", geomean=True))
    assert math.isclose(perm["Sand"], 200.0, rel_tol=1e-9)


def test_pointset_z_stats_and_constructor():
    ps = petekio.PointSet.from_xyz([0.0, 1.0, 2.0], [0.0, 1.0, 2.0], [-100.0, -120.0, -140.0])
    assert len(ps) == 3
    z = ps.z_stats()
    assert math.isclose(z.mean, -120.0)
    assert math.isclose(z.min, -140.0)
    assert math.isclose(z.max, -100.0)


def test_polygonset_from_rings_and_stats():
    poly = petekio.PolygonSet.from_rings([[[0, 0], [10, 0], [10, 10], [0, 10]]])
    assert math.isclose(poly.area(), 100.0)
    assert poly.contains(5.0, 5.0)
    assert len(poly.rings()) == 1


def test_stats_geomean_staticmethod():
    assert math.isclose(petekio.Stats.geomean([2.0, 8.0]), 4.0)
