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


def _las_curves(curves: list[tuple[str, str]], rows: list[tuple[float, ...]]) -> str:
    curve_lines = [" DEPT.M : Measured depth"]
    for mnemonic, unit in curves:
        curve_lines.append(f" {mnemonic}.{unit} :")
    ascii_rows = [" ".join(str(v) for v in row) for row in rows]
    return (
        "~Version\n VERS. 2.0 :\n WRAP. NO :\n"
        "~Well\n STRT.M 100.0 :\n STOP.M 120.0 :\n STEP.M 10.0 :\n NULL. -999.25 :\n"
        "~Curve\n"
        + "\n".join(curve_lines)
        + "\n~ASCII\n"
        + "\n".join(ascii_rows)
        + "\n"
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


def test_project_import_raw_tree_inventory_and_accessors(tmp_path):
    root = tmp_path / "Data"
    _write(root / "Surfaces" / "Top reservoir.irap", _irap())
    _write(root / "Points" / "samples.csv", "x,y,z,poro\n1,2,-3,0.2\n4,5,-6,0.3\n")
    _write(root / "Polygons" / "ModelEdge.geojson", _polygon_geojson())
    _write(root / "Wells" / "99_9-1_A_CompLogs.las", _las())
    _write(root / "Wells" / "crsmeta.xml", '<?xml version="1.0"?><crsmeta><label>ED50</label></crsmeta>')
    _write(root / "WellTops.tops", _petrel_tops())
    _write(root / "Notes" / "readme.txt", "not loadable\n")

    project = petekio.Project.import_data(
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


def test_project_import_accepts_petekio_import_settings(tmp_path):
    root = tmp_path / "Data"
    _write(root / "Wells" / "99_9-1_A_CompLogs.las", _las())

    project = petekio.Project.import_data(
        root,
        settings=petekio.ImportSettings(
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


def test_project_wells_assign_log_strict_same_basis(tmp_path):
    root = tmp_path / "Data"
    _write(
        root / "Wells" / "99_9-1_A_logs.las",
        _las_curves(
            [("PHIE", "v/v"), ("NetSand", "v/v")],
            [(100.0, 0.20, 1.0), (110.0, 0.25, 0.0), (120.0, 0.30, 1.0)],
        ),
    )
    project = petekio.Project.import_data(root)
    logs = project.wells.logs

    result = project.wells.assign_log("PHIE_NET", logs.PHIE * logs.NetSand)

    assert result.summary() == {"created": 1, "skipped": 0, "failed": 0}
    out = project.well("99/9-1").log("PHIE_NET")
    assert out.md() == [100.0, 110.0, 120.0]
    assert out.values() == [0.20, 0.0, 0.30]
    assert any("sidetrack.assign_log(name=PHIE_NET)" in h for h in out.history())
    with pytest.raises(ValueError, match="already exists"):
        project.wells.assign_log("PHIE_NET", logs.PHIE * logs.NetSand)


def test_project_wells_assign_log_requires_or_declares_basis(tmp_path):
    root = tmp_path / "Data"
    _write(
        root / "Wells" / "99_9-1_A_phi.las",
        _las_curves(
            [("PHIE", "v/v")],
            [(100.0, 0.20), (110.0, 0.25), (120.0, 0.30)],
        ),
    )
    _write(
        root / "Wells" / "99_9-1_A_net.las",
        _las_curves(
            [("NetSand", "v/v")],
            [(100.0, 1.0), (115.0, 0.0), (130.0, 1.0)],
        ),
    )
    project = petekio.Project.import_data(root)
    logs = project.wells.logs

    with pytest.raises(ValueError, match="basis mismatch"):
        project.wells.assign_log("PHIE_NET_STRICT", logs.PHIE * logs.NetSand)

    result = project.wells.assign_log(
        "PHIE_NET",
        logs.PHIE * logs.NetSand,
        basis=logs.PHIE,
        interpolation="previous",
    )
    assert result.summary()["created"] == 1
    out = project.well("99/9-1").log("PHIE_NET")
    assert out.md() == [100.0, 110.0, 120.0]
    assert out.values() == [0.20, 0.25, 0.0]

    project.wells.assign_log(
        "PHIE_NET_SPLINE",
        logs.PHIE * logs.NetSand.to_basis(logs.PHIE, interpolation="spline"),
    )
    spline = project.well("99/9-1").log("PHIE_NET_SPLINE")
    assert spline.md() == [100.0, 110.0, 120.0]
    assert len(spline.values()) == 3


def test_project_wells_assign_log_ignores_las_file_boundaries_after_import(tmp_path):
    root = tmp_path / "Data"
    _write(
        root / "Wells" / "99_9-1_A_phi.las",
        _las_curves(
            [("PHIE", "v/v")],
            [(100.0, 0.20), (110.0, 0.25), (120.0, 0.30)],
        ),
    )
    _write(
        root / "Wells" / "99_9-1_A_net.las",
        _las_curves(
            [("NetSand", "v/v")],
            [(100.0, 1.0), (110.0, 0.0), (120.0, 1.0)],
        ),
    )
    project = petekio.Project.import_data(root)
    logs = project.wells.logs

    result = project.wells.assign_log("PHIE_NET", logs.PHIE * logs.NetSand)

    assert result.summary() == {"created": 1, "skipped": 0, "failed": 0}
    out = project.well("99/9-1").log("PHIE_NET")
    assert out.md() == [100.0, 110.0, 120.0]
    assert out.values() == [0.20, 0.0, 0.30]


def test_project_import_accepts_settings_mapping_aliases_and_crs(tmp_path):
    root = tmp_path / "Data"
    _write(root / "Wells" / "99_9-1_A_CompLogs.las", _las())

    project = petekio.Project.import_data(
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


def test_project_import_does_not_false_skip_tops_csv(tmp_path):
    root = tmp_path / "Data"
    well_dir = root / "Wells" / "15_9-A1"
    _write(well_dir / "sample.las", _las())
    _write(well_dir / "tops.csv", "name,md\nTop A,100.0\nBase A,120.0\n")
    _write(root / "Points" / "samples.csv", "x,y,z\n1,2,3\n")

    project = petekio.Project.import_data(root)
    inv = project.inventory()

    assert inv["wells"] == ["15/9-A1"]
    assert inv["points"] == ["samples"]
    assert inv["skipped"] == []
    assert project.well("15/9-A1").top("Top A") is not None


def test_project_import_uses_relative_names_for_duplicate_spatial_stems(tmp_path):
    root = tmp_path / "Data"
    _write(root / "Surfaces" / "Top reservoir.irap", _irap())
    _write(root / "Alternatives" / "Top reservoir.irap", _irap())

    project = petekio.Project.import_data(root)

    assert project.surfaces == ["Alternatives/", "Surfaces/"]
    assert project.surfaces.all_names() == [
        "Alternatives/Top reservoir",
        "Surfaces/Top reservoir",
    ]
    assert project.surfaces.Alternatives == ["Top reservoir"]
    assert project.surface("Alternatives/Top reservoir").stats().count == 4
    assert project.surface("Surfaces/Top reservoir").stats().count == 4


def test_project_import_enriches_irap_points_from_matching_earthvision_topology(tmp_path):
    root = tmp_path / "Data"
    _write(
        root / "Surfaces" / "EarthVision_grid" / "Top Agat.EarthVisionGrid",
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
    )
    _write(
        root / "Surfaces" / "IrapClassic_points" / "Top Agat.IrapClassicPoints",
        """100.0 200.0 -50.0
110.0 200.0 -51.0
100.0 210.0 -52.0
110.0 210.0 -53.0
120.0 210.0 -54.0
""",
    )

    project = petekio.Project.import_data(root)
    pts = project.points["Surfaces/IrapClassic_points/Top Agat"]

    assert pts.attr("column") == [1.0, 2.0, 1.0, 2.0, 3.0]
    assert pts.attr("row") == [1.0, 1.0, 2.0, 2.0, 2.0]
    assert project.points.Surfaces.IrapClassic_points.top_agat.attr("row") == [
        1.0,
        1.0,
        2.0,
        2.0,
        2.0,
    ]
    geom = pts.infer_geometry(tolerance=1e-3, edge="convex_hull")
    assert geom.ncol == 3
    assert geom.nrow == 2


def _top_agat_tree(tmp_path: Path) -> Path:
    root = tmp_path / "Data"
    _write(
        root / "Surfaces" / "EarthVision_grid" / "Top Agat.EarthVisionGrid",
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
    )
    _write(
        root / "Surfaces" / "IrapClassic_points" / "Top Agat.IrapClassicPoints",
        """100.0 200.0 -50.0
110.0 200.0 -51.0
100.0 210.0 -52.0
110.0 210.0 -53.0
120.0 210.0 -54.0
""",
    )
    return root


def test_project_lookup_records_dataset_name(tmp_path):
    root = _top_agat_tree(tmp_path)
    _write(root / "Surfaces" / "Top reservoir.irap", _irap())

    project = petekio.Project.import_data(root)

    pts = project.points.Surfaces.IrapClassic_points["Top Agat"]
    assert pts.name == "Top Agat"
    # The full-path lookup resolves to the same dataset leaf name.
    assert project.points["Surfaces/IrapClassic_points/Top Agat"].name == "Top Agat"
    assert project.surfaces["Top reservoir"].name == "Top reservoir"


def test_dataset_name_propagates_to_derived_objects(tmp_path):
    project = petekio.Project.import_data(_top_agat_tree(tmp_path))
    pts = project.points.Surfaces.IrapClassic_points["Top Agat"]

    geom = pts.infer_geometry(tolerance=1e-3)
    assert geom.name == "Top Agat geometry"

    surf = pts.to_surface(geom)
    assert surf.name == "Top Agat"
    assert surf.geometry.name == "Top Agat geometry"

    structured = pts.to_structured_surface(tolerance=1e-3)
    assert structured.name == "Top Agat"
    assert structured.to_tri_surface().name == "Top Agat"
    assert structured.to_points().name == "Top Agat"

    labelled, report = pts.detect_topology()
    assert report.verified
    assert labelled.name == "Top Agat"


def test_project_load_pproj_delegates_to_geodata_open(tmp_path):
    geo = petekio.GeoData(unit="m")
    geo.load_surface("top", str(_write(tmp_path / "top.irap", _irap())))
    pproj = tmp_path / "field.pproj"
    geo.save(str(pproj))

    project = petekio.Project.load(pproj)

    assert isinstance(project.geodata, petekio.GeoData)
    assert project.surface("top").stats().count == 4
    assert project.inventory()["surfaces"] == ["top"]


def test_project_load_rejects_raw_source_directory(tmp_path):
    root = tmp_path / "Data"
    root.mkdir()

    with pytest.raises(ValueError, match="Use Project.import_data"):
        petekio.Project.load(root)


def test_project_import_rejects_pproj(tmp_path):
    geo = petekio.GeoData(unit="m")
    pproj = tmp_path / "field.pproj"
    geo.save(str(pproj))

    with pytest.raises(ValueError, match="raw source directory"):
        petekio.Project.import_data(pproj)


def test_project_save_writes_compact_project(tmp_path):
    root = tmp_path / "Data"
    _write(root / "Surfaces" / "Top reservoir.irap", _irap())
    project = petekio.Project.import_data(root)

    pproj = tmp_path / "field.pproj"
    project.save(pproj)
    reopened = petekio.Project.load(pproj)

    assert reopened.surfaces == ["Top reservoir"]
    assert reopened.surface("Top reservoir").stats().count == 4

    with pytest.raises(ValueError, match=".pproj"):
        project.save(tmp_path / "field")


def test_project_folder_navigation_and_object_management(tmp_path):
    root = tmp_path / "Data"
    _write(root / "Surfaces" / "Top reservoir.irap", _irap())
    _write(root / "Points" / "samples.csv", "x,y,z,poro\n1,2,-3,0.2\n4,5,-6,0.3\n")
    _write(root / "Polygons" / "ModelEdge.geojson", _polygon_geojson())
    _write(root / "Wells" / "99_9-1_A_CompLogs.las", _las())
    _write(root / "WellTops.tops", _petrel_tops())

    project = petekio.Project.import_data(root)

    project.rename_surface("Top reservoir", "structure/top agat")
    assert project.surfaces == ["structure/"]
    assert project.structures == ["structure/"]
    assert project.surfaces.structure == ["top agat"]
    assert project.surfaces.structure.top_agat.stats().count == 4
    assert project.surfaces.top_agat.stats().count == 4
    assert project.inventory()["surfaces"] == ["structure/top agat"]

    project.rename_points("samples", "data/samples")
    project.rename_polygons("ModelEdge", "maps/model edge")
    project.rename_well("99/9-1", "wells/A1")
    project.rename_tops("WellTops", "tops/main")

    assert project.points == ["data/"]
    assert project.points.data.samples.stats("poro").count == 2
    assert project.polygons.maps.model_edge.contains(0.5, 0.5)
    assert project.well("wells/A1") is not None
    assert project.tops == ["tops/"]
    assert list(project.tops.tops.main["surface"]) == ["Top A", "Base A"]

    pproj = tmp_path / "managed.pproj"
    project.save(pproj)
    reopened = petekio.Project.load(pproj)
    assert reopened.surfaces == ["structure/"]
    assert reopened.surfaces.top_agat.stats().count == 4
    assert reopened.points.data.samples.stats("poro").count == 2

    reopened.delete_surface("top agat")
    reopened.delete_points("data/samples")
    reopened.delete_polygons("maps/model edge")
    reopened.delete_well("wells/A1")

    assert reopened.inventory()["counts"]["surfaces"] == 0
    assert reopened.inventory()["counts"]["points"] == 0
    assert reopened.inventory()["counts"]["polygons"] == 0
    assert reopened.inventory()["counts"]["wells"] == 0


def test_project_rename_and_delete_errors_are_loud(tmp_path):
    root = tmp_path / "Data"
    _write(root / "Surfaces" / "Top A.irap", _irap())
    _write(root / "Surfaces" / "Top B.irap", _irap())
    project = petekio.Project.import_data(root)

    with pytest.raises(ValueError, match="already exists"):
        project.rename_surface("Top A", "Top B")
    with pytest.raises(KeyError):
        project.delete_surface("missing")
    with pytest.raises(ValueError, match="unsupported"):
        project.rename("cube", "Top A", "x")
