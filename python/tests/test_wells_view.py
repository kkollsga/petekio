"""Standalone ``well.view()`` — the WellLogBundle producer + logs-only viewer
session (petekio's slice of the well-correlation seam,
``petekSuite/dev-docs/designs/well-log-bundle-seam.md``; wire format codified in
``petektools/viewer/SCHEMA.md``).

Covers: bit-exact f32 lane encoding + NaN policy, the bundle shape vs the viewer
reference fixture (``petektools/viewer/_wells.py``), TVD derivation (vertical
assumption + trajectory), tops opt-in + top→down ordering validation,
missing-pick passthrough, the "no ties standalone" rule, and a live viewer
round-trip when the petektools wheel is importable. Fixtures are synthetic
(fictional ``99/…`` wells, ``431000/6521000`` coords), hand-authored to spec.
"""
import base64
import math
import os
import struct

import pytest

import petekio
from petekio._viewer import build_well_log_bundle, encode_lane

# The viewer unit is an OPTIONAL runtime dependency; its reference fixture is the
# round-trip target. Skip the viewer-dependent tests if the wheel isn't present.
_wells = pytest.importorskip("petektools.viewer._wells")


# --------------------------------------------------------------------------- #
# fixtures                                                                     #
# --------------------------------------------------------------------------- #
def _well_dir(tmp_path, wid="99_3-1", tops="Upper Sand,2000\nMid Shale,2002\nBase,2004\n"):
    """A synthetic single vertical well: a 5-sample LAS (PHIE/SW/FACIES) + a
    formation-tops CSV. KB=25 m so TVDSS = MD − 25 under the vertical assumption."""
    las = (
        "~Version\n VERS. 2.0 :\n WRAP. NO :\n"
        "~Well\n STRT.M 2000.0 :\n STOP.M 2004.0 :\n STEP.M 1.0 :\n NULL. -999.25 :\n"
        "~Curve\n DEPTH.M :\n PHIE.v/v :\n SW.v/v :\n FACIES.NONE :\n"
        "~ASCII\n"
        "2000.0 0.20 0.30 1\n"
        "2001.0 0.05 0.80 0\n"
        "2002.0 0.22 0.28 1\n"
        "2003.0 0.25 0.20 1\n"
        "2004.0 0.18 0.35 1\n"
    )
    (tmp_path / f"{wid}.las").write_text(las)
    (tmp_path / f"{wid}.csv").write_text("name,md\n" + tops)
    return str(tmp_path)


def _geo(tmp_path, wid="99/3-1", fid="99_3-1", tops=None):
    geo = petekio.GeoData(unit="m")
    kw = {} if tops is None else {"tops": tops}
    geo.load_well(wid, head=(431000.0, 6521000.0), kb=25.0, files=_well_dir(tmp_path, fid, **kw))
    return geo


def _decode(lane):
    raw = base64.b64decode(lane["data"])
    assert lane["dtype"] == "f32"
    n = len(raw) // 4
    assert lane["shape"] == [n]
    return list(struct.unpack("<%df" % n, raw))


# --------------------------------------------------------------------------- #
# lane encoding — bit-exact f32 + NaN policy                                   #
# --------------------------------------------------------------------------- #
def test_encode_lane_bit_exact_f32():
    xs = [0.0, 0.125, -1.5, 3.4028235e38, 0.1]
    lane = encode_lane(xs)
    raw = base64.b64decode(lane["data"])
    assert lane == {"dtype": "f32", "shape": [5], "data": lane["data"]}
    got = struct.unpack("<5f", raw)
    for a, b in zip(xs, got):
        # bit-exact against a single f32 round-trip of the same double
        assert struct.pack("<f", a) == struct.pack("<f", b)


def test_encode_lane_nan_policy_is_canonical():
    # None AND float nan AND inf all pack as the canonical quiet NaN 0x7FC00000.
    lane = encode_lane([None, float("nan"), float("inf"), -float("inf"), 1.0])
    raw = base64.b64decode(lane["data"])
    words = struct.unpack("<5I", raw)
    assert words[:4] == (0x7FC00000, 0x7FC00000, 0x7FC00000, 0x7FC00000)
    assert words[4] == struct.unpack("<I", struct.pack("<f", 1.0))[0]


def test_encode_lane_byte_identical_to_reference():
    # Byte-identity with the viewer's own reference encoder is the seam-twin anchor.
    xs = [0.235, 0.075, None, 0.4, float("nan"), 0.0]
    assert encode_lane(xs) == _wells.encode_lane(xs)


# --------------------------------------------------------------------------- #
# bundle shape vs the reference fixture                                        #
# --------------------------------------------------------------------------- #
def test_bundle_matches_reference_shape(tmp_path):
    geo = _geo(tmp_path)
    b = geo.well("99/3-1").view(serve=False, tops=True).bundle()

    ref = _wells.build_well_log_bundle()
    assert b["kind"] == ref["kind"] == "wells_logs"
    assert b["schema_version"] == ref["schema_version"] == 4
    assert set(b.keys()) == set(ref.keys())

    lw = b["wells"][0]
    rw = ref["wells"][0]
    # Same LogWell keys EXCEPT `ties` (model context only — never standalone).
    assert set(lw.keys()) == set(rw.keys()) - {"ties"}
    assert "ties" not in lw

    # lanes are valid v3 f32 blocks of matching length
    md = _decode(lw["md_m"])
    tvd = _decode(lw["tvd_m"])
    assert len(md) == len(tvd) == 5
    for c in lw["curves"]:
        vals = _decode(c["values"])
        assert len(vals) == len(md)
        assert set(c.keys()) >= {"mnemonic", "display_name", "unit", "kind", "values"}


def test_curve_extras_flag_cutoff_range_codes(tmp_path):
    geo = _geo(tmp_path)
    lw = geo.well("99/3-1").view(serve=False).bundle()["wells"][0]
    by = {c["mnemonic"]: c for c in lw["curves"]}
    # PHIE: continuous, canonicalized name, cutoff on effective porosity, range.
    assert by["PHIE"]["kind"] == "continuous"
    assert by["PHIE"]["cutoff"] == pytest.approx(0.08)
    assert by["PHIE"]["range"]["max"] == pytest.approx(0.25)
    assert by["PHIE"]["unit"] == "v/v"
    # FACIES: flag strip with integer codes; no range.
    assert by["FACIES"]["kind"] == "flag"
    assert by["FACIES"]["codes"] == {"0": "0", "1": "1"}
    assert "range" not in by["FACIES"]


def test_mnemonic_canonicalization(tmp_path):
    # petekio is the family name authority: raw SUWI/PHI resolve to canonical.
    assert petekio.canonical_mnemonic("suwi") == "SW"
    assert petekio.canonical_mnemonic("PHI") == "PHIE"


# --------------------------------------------------------------------------- #
# TVD derivation                                                               #
# --------------------------------------------------------------------------- #
def test_tvd_vertical_assumption(tmp_path):
    # No trajectory loaded → TVDSS = MD − KB (documented vertical assumption).
    geo = _geo(tmp_path)
    lw = geo.well("99/3-1").view(serve=False).bundle()["wells"][0]
    md = _decode(lw["md_m"])
    tvd = _decode(lw["tvd_m"])
    assert lw["datum_m"] == pytest.approx(25.0)
    for m, t in zip(md, tvd):
        assert t == pytest.approx(m - 25.0)


# --------------------------------------------------------------------------- #
# tops opt-in + ordering                                                       #
# --------------------------------------------------------------------------- #
def test_tops_omitted_by_default(tmp_path):
    geo = _geo(tmp_path)
    lw = geo.well("99/3-1").view(serve=False).bundle()["wells"][0]
    assert "tops" not in lw and "zones" not in lw


def test_tops_opt_in_top_down(tmp_path):
    geo = _geo(tmp_path)
    b = geo.well("99/3-1").view(serve=False, tops=True).bundle()
    lw = b["wells"][0]
    tvds = [t["tvd_m"] for t in lw["tops"]]
    assert tvds == sorted(tvds)  # top→down
    assert b["flatten_default"] == lw["tops"][0]["horizon"]
    # zones band between consecutive tops, in the zone's tvd frame
    assert all(z["base_tvd_m"] >= z["top_tvd_m"] for z in lw["zones"])


def test_tops_filter_subset(tmp_path):
    geo = _geo(tmp_path)
    lw = geo.well("99/3-1").view(serve=False, tops=["Upper Sand"]).bundle()["wells"][0]
    assert [t["horizon"] for t in lw["tops"]] == ["Upper Sand"]


def test_unsorted_tops_raise_loudly():
    # A raw well with an overturned (decreasing-TVD) stack must fail loudly.
    raw = {
        "id": "99/x-1", "display_name": "99/x-1", "x": 431000.0, "y": 6521000.0,
        "datum_m": 25.0, "md": [2000.0, 2001.0], "tvd": [1975.0, 1976.0],
        "curves": [{"mnemonic": "PHIE", "canonical": "PHIE", "unit": "v/v",
                    "core": False, "values": [0.2, 0.1]}],
        "zones": [
            {"name": "A", "top_md": 2000.0, "base_md": 2001.0, "top_tvd": 1980.0, "base_tvd": 1985.0},
            {"name": "B", "top_md": 2001.0, "base_md": 2002.0, "top_tvd": 1975.0, "base_tvd": 1978.0},
        ],
    }
    with pytest.raises(ValueError, match="not sorted top->down"):
        build_well_log_bundle([raw], tops=True)


# --------------------------------------------------------------------------- #
# missing-pick passthrough (multi-well session)                               #
# --------------------------------------------------------------------------- #
def test_missing_pick_passthrough(tmp_path):
    geo = petekio.GeoData(unit="m")
    a = tmp_path / "a"
    b = tmp_path / "b"
    a.mkdir()
    b.mkdir()
    # well A has the Mid Shale formation; well B does not (missing pick).
    geo.load_well("99/3-1", head=(431000.0, 6521000.0), kb=25.0,
                  files=_well_dir(a, "99_3-1", tops="Upper Sand,2000\nMid Shale,2002\nBase,2004\n"))
    geo.load_well("99/6-1", head=(432000.0, 6521500.0), kb=25.0,
                  files=_well_dir(b, "99_6-1", tops="Upper Sand,2000\nBase,2004\n"))

    b_ = geo.wells.view(serve=False, tops=True).bundle()
    wells = {w["id"]: w for w in b_["wells"]}
    assert "Mid Shale" in {t["horizon"] for t in wells["99/3-1"]["tops"]}
    assert "Mid Shale" not in {t["horizon"] for t in wells["99/6-1"]["tops"]}
    # neither carries ties standalone
    assert all("ties" not in w for w in b_["wells"])


# --------------------------------------------------------------------------- #
# live viewer round-trip                                                       #
# --------------------------------------------------------------------------- #
def test_live_viewer_roundtrip(tmp_path):
    viewer = pytest.importorskip("petektools.viewer")
    geo = _geo(tmp_path)
    sess = geo.well("99/3-1").view(serve=False, tops=True)

    # 1) self-contained HTML export accepts our bundle exactly like the fixture
    ours = str(tmp_path / "ours.html")
    sess.save(ours)
    html = open(ours).read()
    assert "wells_logs" in html and len(html) > 10_000

    ref = str(tmp_path / "ref.html")
    viewer.save_view(_wells.build_well_log_bundle(), ref)
    assert os.path.getsize(ref) > 10_000

    # 2) the live server builds (non-blocking) over our bundle
    httpd, url = viewer.build_server(sess.bundle())
    try:
        assert url.startswith("http://127.0.0.1:")
    finally:
        httpd.server_close()


def test_template_is_additive_validated_and_bound_project_callable(tmp_path):
    from petektools import viewer

    if not hasattr(viewer, "CorrelationTemplate"):
        pytest.skip("installed optional petektools predates correlation templates")

    geo = _geo(tmp_path)
    plain = geo.well("99/3-1").view(serve=False).bundle()
    assert "template" not in plain

    template = viewer.CorrelationTemplate("reservoir").add_track(
        viewer.CorrelationTrack("phi", minimum=0, maximum=0.35).curve("PHIE")
    )
    direct = geo.well("99/3-1").view(template=template, serve=False).bundle()
    assert direct["template"] == template.to_dict()

    missing = viewer.CorrelationTemplate("missing").add_track(
        viewer.CorrelationTrack("absent").curve("DOES_NOT_EXIST")
    )
    with pytest.raises(ValueError, match="absent from every well"):
        geo.wells.view(template=missing.to_dict(), serve=False)

    path = tmp_path / "bound.pproj"
    geo.save(str(path))
    project = petekio.Project.load(path)
    bound = project.templates.add(template)
    via_collection = project.wells.view(template=bound, serve=False).bundle()
    via_callable = project.templates.reservoir(wells=["99/3-1"], serve=False).bundle()
    assert via_collection["template"] == template.to_dict()
    assert via_callable["template"] == template.to_dict()

    html = tmp_path / "templated.html"
    project.templates.reservoir(wells="99/3-1", save=str(html))
    assert html.exists() and "CorrelationTemplate" in html.read_text()
