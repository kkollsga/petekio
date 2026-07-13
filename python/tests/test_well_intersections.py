from __future__ import annotations

import math
from pathlib import Path

import petekio
import pytest


def _wellpath(path: Path, x: float, *, horizontal_at: float | None = None) -> Path:
    path.mkdir(parents=True, exist_ok=True)
    if horizontal_at is None:
        rows = (
            f"0 {x} 0 0 0 0 0 0 0 0 0\n"
            f"20 {x} 0 -20 20 0 0 0 0 0 0\n"
        )
    else:
        rows = (
            f"0 {x - 5} 0 {horizontal_at} {-horizontal_at} 0 0 90 90 0 90\n"
            f"10 {x + 5} 0 {horizontal_at} {-horizontal_at} 10 0 90 90 0 90\n"
        )
    (path / "trace.wellpath").write_text(
        "# WELL TRACE FROM PETREL\n"
        f"# WELL HEAD X-COORDINATE: {x} (m)\n"
        "# WELL HEAD Y-COORDINATE: 0 (m)\n"
        "# WELL DATUM (KB, Kelly bushing, from MSL): 0 (m)\n"
        "=====\n"
        "MD X Y Z TVD DX DY AZIM_TN INCL DLS AZIM_GN\n"
        + rows
    )
    return path


def _inventory(wells: list[str]) -> dict[str, object]:
    return {
        "source": None,
        "surfaces": [],
        "wells": wells,
        "tops": [],
        "points": [],
        "polygons": [],
        "sidecars": [],
        "skipped": [],
        "counts": {"wells": len(wells)},
    }


def _surface(z: float) -> petekio.Surface:
    geometry = petekio.GridGeometry(-10, -10, 10, 10, 3, 3)
    return petekio.Surface.constant(geometry, z)


def test_typed_intersection_single_multiple_identity_and_top_mutation(tmp_path):
    trajectory = petekio.Trajectory.from_stations(
        [(0, 0, 0), (20, 0, 0)], head=(0, 0), kb=0
    )
    hit = trajectory.intersection(_surface(-5))
    assert isinstance(hit, petekio.SurfaceIntersection)
    assert math.isclose(hit.md, 5.0, abs_tol=1e-3)
    assert hit.xyz == pytest.approx((0.0, 0.0, -5.0), abs=1e-3)
    assert hit["md"] == hit.md
    assert hit.to_dict()["surface"] is None
    assert trajectory.intersection(_surface(5)) is None

    geo = petekio.GeoData(unit="m")
    geo.load_well("W1", files=str(_wellpath(tmp_path / "W1", 0)))
    bore = geo.well("W1").sidetrack("")
    named_hit = bore.intersection(_surface(-5))
    assert named_hit.well == "W1" and named_hit.bore == ""
    bore.add_top("Top A", named_hit)
    assert bore.tops() == [("Top A", pytest.approx(5.0, abs=1e-3))]
    with pytest.raises(ValueError, match="already exists"):
        bore.add_top("top a", 6.0)
    bore.replace_top("Top A", 6.0)
    assert bore.tops()[0][1] == 6.0
    bore.remove_top("top a")
    assert bore.tops() == []


def test_project_aggregate_atomic_well_tops_and_pproj_roundtrip(tmp_path):
    geo = petekio.GeoData(unit="m")
    geo.load_well("W1", files=str(_wellpath(tmp_path / "W1", 0)))
    geo.load_well("W2", files=str(_wellpath(tmp_path / "W2", 100)))
    project = petekio.Project(geo, inventory=_inventory(["W1", "W2"]))

    result = project.wells.intersection(_surface(-5))
    assert isinstance(result, petekio.WellIntersectionSet)
    assert result.summary() == {"hits": 1, "skipped": 1, "failed": 0}
    assert result.hits[0].well == "W1"
    assert result.skipped[0]["reason"] == "outside_or_no_intersection"

    other_geo = petekio.GeoData(unit="m")
    other_geo.load_well("W1", files=str(_wellpath(tmp_path / "OtherW1", 0)))
    other = petekio.Project(other_geo, inventory=_inventory(["W1"]))
    with pytest.raises(ValueError, match="different project"):
        other.well_tops["Top A"] = result

    project.well_tops["Reservoir/Top A"] = result
    top_set = project.well_tops["Reservoir/Top A"]
    assert isinstance(top_set, petekio.WellTopSet)
    assert top_set.summary() == {"picks": 1, "wells": 1, "bores": 1}
    assert top_set[0]["well"] == "W1"
    assert project.well_tops.Reservoir["Top A"].name == "Reservoir/Top A"

    # Replacement is complete: a stale same-name W2 pick is removed because W2
    # is outside the new result, while W1 moves to the new crossing.
    geo.well("W2").sidetrack("").add_top("Reservoir/Top A", 4.0)
    project.well_tops["Reservoir/Top A"] = project.wells.intersection(_surface(-7))
    rows = project.well_tops["Reservoir/Top A"].rows
    assert len(rows) == 1 and rows[0]["well"] == "W1"
    assert rows[0]["md"] == pytest.approx(7.0, abs=1e-3)

    # A failed coplanar bore blocks assignment before any existing pick changes.
    geo.load_well("W3", files=str(_wellpath(tmp_path / "W3", 0, horizontal_at=-5)))
    complete = petekio.Project(geo, inventory=_inventory(["W1", "W2", "W3"]))
    failed = complete.wells.intersection(_surface(-5))
    assert failed.summary()["failed"] == 1
    before = complete.well_tops["Reservoir/Top A"].rows
    with pytest.raises(ValueError, match="blocked"):
        complete.well_tops["Reservoir/Top A"] = failed
    assert complete.well_tops["Reservoir/Top A"].rows == before

    out = tmp_path / "field.pproj"
    complete.save(out)
    reopened = petekio.Project.load(out)
    assert reopened.well_tops["Reservoir/Top A"].rows == before
    assert reopened.tops == []  # source top-set tables remain a separate API
    del reopened.well_tops["Reservoir/Top A"]
    assert reopened.well_tops.all_names() == []


def test_project_aggregate_keeps_each_multibore_hit(tmp_path):
    template = _wellpath(tmp_path / "template", 0) / "trace.wellpath"
    multi = tmp_path / "multi"
    multi.mkdir()
    (multi / "WM_A.wellpath").write_text(template.read_text())
    (multi / "WM_B.wellpath").write_text(template.read_text())

    geo = petekio.GeoData(unit="m")
    geo.load_well("WM", files=str(multi))
    project = petekio.Project(geo, inventory=_inventory(["WM"]))
    result = project.wells.intersection(_surface(-5))
    assert result.summary() == {"hits": 2, "skipped": 0, "failed": 0}
    assert {hit.bore for hit in result.hits} == {"A", "B"}

    project.well_tops["Top multi"] = result
    rows = project.well_tops["Top multi"].rows
    assert {(row["well"], row["bore"]) for row in rows} == {("WM", "A"), ("WM", "B")}
