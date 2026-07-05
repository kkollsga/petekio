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
