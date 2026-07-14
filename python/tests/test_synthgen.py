"""The examples synthetic-data generator must produce files petekIO ingests —
this round-trip is the generator's safety net (no pandas needed)."""

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "examples"))

import petekio  # noqa: E402
import synthgen  # noqa: E402

WELLS = ["25/1-1", "25/1-2", "25/1-3"]


def _load(tmp_path):
    paths = synthgen.make_field(tmp_path)
    geo = petekio.GeoData(unit="m")
    for wid in WELLS:
        geo.load_well(wid, files=str(paths["wells_dir"]))
    assert geo.load_well_tops(str(paths["tops"])) > 0
    return geo


def test_synthgen_round_trip(tmp_path):
    geo = _load(tmp_path)
    # Column merged shallow→deep across all wells; stacked lobes both present.
    so = geo.strat_order
    assert so[0] == "Top Shale" and so[-1] == "Base Reservoir"
    assert {"Mid Sand", "Lower Sand A", "Lower Sand B"} <= set(so)
    # The multi-bore well resolved to bores A + B.
    assert {"A", "B"} <= set(geo.well("25/1-1").bores())
    # Mid Sand pinches out in 25/1-3 → zero thickness there.
    zones = {n: (t, b) for n, t, b in geo.well("25/1-3").sidetrack("").zones()}
    t, b = zones["Mid Sand"]
    assert b - t == 0.0


def test_synthgen_hint_reorders_coincident_lobes(tmp_path):
    # Lower Sand A/B are coincident in every well → order is a tiebreak; a hint
    # flips it (real positions would win, but here there are none).
    paths = synthgen.make_field(tmp_path)
    geo = petekio.GeoData(unit="m")
    geo.strat_hint("Lower Sand A < Lower Sand B")  # A above B
    for wid in WELLS:
        geo.load_well(wid, files=str(paths["wells_dir"]))
    geo.load_well_tops(str(paths["tops"]))
    so = geo.strat_order
    assert so.index("Lower Sand A") < so.index("Lower Sand B")


def test_viewer_design_fixture_has_positioned_deviated_multibore_well(tmp_path):
    paths = synthgen.make_viewer_design_field(tmp_path)
    geo = petekio.GeoData(unit="m")
    geo.load_well("25/1-1", files=str(paths["wells_dir"]))
    well = geo.well("25/1-1")

    assert well.head == (1000.0, 5000.0)
    assert well.is_multibore is True
    assert set(well.bores()) == {"", "A", "B"}

    bore_a = well.sidetrack("A")
    first_md, last_md = bore_a.md_range()
    first = bore_a.xyz(first_md)
    last = bore_a.xyz(last_md)
    assert first[:2] == (1000.0, 5000.0)
    assert last[0] - first[0] > 50.0
    assert last[1] - first[1] > 50.0

    bore_b = well.sidetrack("B")
    first_md, last_md = bore_b.md_range()
    assert bore_b.xyz(first_md)[:2] == bore_b.xyz(last_md)[:2]

    geo.load_well("25/1-2", files=str(paths["wells_dir"]))
    assert geo.well("25/1-2").head == (3000.0, 5200.0)


def test_viewer_design_fixture_has_rotated_typed_surface_lanes(tmp_path):
    paths = synthgen.make_viewer_design_field(tmp_path)
    surface = petekio.Surface.load_irap_classic(str(paths["surface"]))
    expected = paths["surface_geometry"]

    geom = surface.geometry
    assert (geom.ncol, geom.nrow) == (7, 6)
    assert geom.rotation_deg == 27.0
    assert (geom.xori, geom.yori, geom.xinc, geom.yinc) == (
        expected["xori"],
        expected["yori"],
        expected["xinc"],
        expected["yinc"],
    )

    attributes = paths["attributes"]
    continuous = {
        name for name, lane in attributes.items() if lane["kind"] == "continuous"
    }
    assert continuous == {
        "gross_thickness",
        "porosity",
    }
    assert attributes["gross_thickness"]["unit"] == "m"
    assert attributes["porosity"]["unit"] == "fraction"
    assert attributes["facies"]["kind"] == "categorical"
    assert attributes["facies"]["unit"] is None
    assert attributes["facies"]["codes"] == {1: "Shale", 2: "Sand", 3: "Silt"}

    for name, lane in attributes.items():
        lane_surface = petekio.Surface.load_irap_classic(str(lane["path"]))
        lane_geom = lane_surface.geometry
        assert (lane_geom.ncol, lane_geom.nrow) == (geom.ncol, geom.nrow)
        assert lane_geom.rotation_deg == geom.rotation_deg
        surface.set_attr(name, lane_surface)

    assert set(surface.attr_names()) == set(attributes)
    facies = surface.attr("facies").stats()
    assert (facies.count, facies.min, facies.max) == (42, 1.0, 3.0)
