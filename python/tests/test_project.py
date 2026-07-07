from pathlib import Path

import petekio
import pytest


def _write(path: Path, text: str) -> Path:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text)
    return path


def _irap() -> str:
    return "-996 2 10 10\n0 10 0 10\n2 0 0 0\n0 0 0 0 0 0 0\n1 2 3 4\n"


def _las() -> str:
    return (
        "~Version\n VERS. 2.0 :\n WRAP. NO :\n"
        "~Well\n STRT.M 100.0 :\n STOP.M 120.0 :\n STEP.M 10.0 :\n NULL. -999.25 :\n"
        "~Curve\n DEPT.M :\n PHIE_2025.m3/m3 :\n"
        "~ASCII\n100.0 0.20\n110.0 0.25\n120.0 0.30\n"
    )


def _petrel_tops(well: str = "99/9-1") -> str:
    return (
        "# Petrel well tops\nVERSION 2\nBEGIN HEADER\n"
        "X\nY\nZ\nTWT\nTWT2\nage\nMD\nPVD\nType\nSurface\nWell\nEND HEADER\n"
        f'1 2 -1 -999 -999 -999 105.0 -1 Horizon "Top A" "{well}"\n'
        f'1 2 -1 -999 -999 -999 115.0 -1 Horizon "Base A" "{well}"\n'
    )


def _polygon_geojson() -> str:
    return (
        '{"type":"FeatureCollection","features":[{"type":"Feature",'
        '"geometry":{"type":"Polygon","coordinates":[[[0,0],[1,0],[1,1],[0,1],[0,0]]]},'
        '"properties":{}}]}'
    )


def test_project_load_raw_tree_inventory_and_accessors(tmp_path):
    root = tmp_path / "Data"
    _write(root / "Surfaces" / "Top reservoir.irap", _irap())
    _write(root / "Points" / "samples.csv", "x,y,z,poro\n1,2,-3,0.2\n4,5,-6,0.3\n")
    _write(root / "Polygons" / "ModelEdge.geojson", _polygon_geojson())
    _write(root / "Wells" / "99_9-1_A_CompLogs.las", _las())
    _write(root / "Wells" / "crsmeta.xml", '<?xml version="1.0"?><crsmeta><label>ED50</label></crsmeta>')
    _write(root / "WellTops.tops", _petrel_tops())
    _write(root / "Notes" / "readme.txt", "not loadable\n")

    project = petekio.Project.load(
        root,
        aliases={"por": ["PHIE_2025"]},
        crs="ED50",
        settings={"unit": "m"},
    )

    inv = project.inventory()
    assert inv["counts"] == {
        "surfaces": 1,
        "wells": 1,
        "tops": 1,
        "points": 1,
        "polygons": 1,
        "skipped": 1,
    }
    assert inv["surfaces"] == ["Top reservoir"]
    assert inv["points"] == ["samples"]
    assert inv["polygons"] == ["ModelEdge"]
    assert inv["wells"] == ["99/9-1"]
    assert inv["tops"] == ["WellTops"]
    assert inv["sidecars"] == ["Wells/crsmeta.xml"]
    assert inv["skipped"][0]["reason"] == "unsupported_format"
    assert all(item["path"] != "Wells/crsmeta.xml" for item in inv["skipped"])

    assert project.surfaces == ["Top reservoir"]
    assert list(project.surfaces) == ["Top reservoir"]
    assert project.surfaces[0] == "Top reservoir"
    assert project.surface("Top reservoir").stats().count == 4
    assert project.surfaces["Top reservoir"].stats().count == 4
    assert project.points["samples"].stats("poro").count == 2
    assert project.polygons["ModelEdge"].contains(0.5, 0.5)

    assert project.wells == ["99/9-1"]
    assert list(project.wells) == ["99/9-1"]
    assert project.wells[0] == "99/9-1"
    assert project.wells.logs == ["por"]
    assert str(project.wells.logs) == "['por']"
    assert list(project.wells.logs) == ["por"]
    assert project.wells.logs[0] == "por"
    assert project.wells["99/9-1"].logs == ["por"]
    assert project.wells._99_9_1.logs == ["por"]
    assert project.wells.logs.por.name == "por"

    assert project.tops == ["WellTops"]
    assert str(project.tops) == "['WellTops']"
    pytest.importorskip("pandas")
    tops = project.tops["well tops"]
    assert list(tops["surface"]) == ["Top A", "Base A"]
    assert list(tops["well"]) == ["99/9-1", "99/9-1"]
    assert list(tops["md"]) == [105.0, 115.0]
    assert project.tops["WellTops"].equals(tops)

    well = project.well("99/9-1")
    assert well is not None
    assert well.top("Top A") is not None
    assert well.log("por").stats().count == 3
    assert well.crs == "ED50"


def test_project_load_accepts_petekio_load_settings(tmp_path):
    root = tmp_path / "Data"
    _write(root / "Wells" / "99_9-1_A_CompLogs.las", _las())

    project = petekio.Project.load(
        root,
        settings=petekio.LoadSettings(
            crs="EPSG:32631",
            aliases={"por": ["PHIE_2025"]},
            unit="m",
        ),
    )

    inv = project.inventory()
    assert project.crs == "EPSG:32631"
    assert project.aliases == {"por": ["PHIE_2025"]}
    assert project.settings == {"unit": "m"}
    assert inv["crs"] == "EPSG:32631"
    assert inv["aliases"] == {"por": ["PHIE_2025"]}
    assert project.well("99/9-1").log("por").stats().count == 3


def test_project_load_accepts_settings_mapping_aliases_and_crs(tmp_path):
    root = tmp_path / "Data"
    _write(root / "Wells" / "99_9-1_A_CompLogs.las", _las())

    project = petekio.Project.load(
        root,
        settings={
            "crs": "EPSG:32631",
            "aliases": {"por": ["PHIE_2025"]},
            "unit": "m",
        },
    )

    assert project.crs == "EPSG:32631"
    assert project.aliases == {"por": ["PHIE_2025"]}
    assert project.settings == {"unit": "m"}
    assert project.well("99/9-1").log("por").stats().count == 3


def test_project_load_does_not_false_skip_tops_csv(tmp_path):
    root = tmp_path / "Data"
    well_dir = root / "Wells" / "15_9-A1"
    _write(well_dir / "sample.las", _las())
    _write(well_dir / "tops.csv", "name,md\nTop A,100.0\nBase A,120.0\n")
    _write(root / "Points" / "samples.csv", "x,y,z\n1,2,3\n")

    project = petekio.Project.load(root)
    inv = project.inventory()

    assert inv["wells"] == ["15/9-A1"]
    assert inv["points"] == ["samples"]
    assert inv["skipped"] == []
    assert project.well("15/9-A1").top("Top A") is not None


def test_project_load_uses_relative_names_for_duplicate_spatial_stems(tmp_path):
    root = tmp_path / "Data"
    _write(root / "Surfaces" / "Top reservoir.irap", _irap())
    _write(root / "Alternatives" / "Top reservoir.irap", _irap())

    project = petekio.Project.load(root)

    assert project.surfaces == ["Alternatives.Top reservoir", "Surfaces.Top reservoir"]
    assert project.surface("Alternatives.Top reservoir").stats().count == 4
    assert project.surface("Surfaces.Top reservoir").stats().count == 4


def test_project_load_pproj_delegates_to_geodata_open(tmp_path):
    geo = petekio.GeoData(unit="m")
    geo.load_surface("top", str(_write(tmp_path / "top.irap", _irap())))
    pproj = tmp_path / "field.pproj"
    geo.save(str(pproj))

    project = petekio.Project.load(pproj)

    assert isinstance(project.geodata, petekio.GeoData)
    assert project.surface("top").stats().count == 4
    assert project.inventory()["surfaces"] == ["top"]
