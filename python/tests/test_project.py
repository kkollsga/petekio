import math
import builtins
import json
from pathlib import Path

import petekio
import pytest


def _correlation_template_dict(name: str = "reservoir", **extra):
    return {
        "spec": "CorrelationTemplate",
        "schema_version": 1,
        "name": name,
        "tracks": [],
        **extra,
    }


def test_project_display_metadata_round_trips(tmp_path):
    project = petekio.Project(
        petekio.GeoData(unit="m"),
        display_name="Synthetic appraisal",
        crs="EPSG:23031 / local datum note",
    )
    path = tmp_path / "metadata.pproj"
    project.save(path)

    info = petekio.GeoData.inspect(str(path))
    assert info["display_name"] == "Synthetic appraisal"
    assert info["crs"] == "EPSG:23031 / local datum note"
    assert info["unit"] == "Metres"

    reopened = petekio.Project.load(path)
    assert reopened.display_name == "Synthetic appraisal"
    assert reopened.crs == "EPSG:23031 / local datum note"
    assert reopened.unit == "m"
    assert reopened.inventory()["crs"] == "EPSG:23031 / local datum note"


def test_project_display_metadata_rejects_noncanonical_strings():
    project = petekio.Project(petekio.GeoData(unit="m"))
    with pytest.raises(ValueError, match="non-empty, trimmed string"):
        project.display_name = " \t"
    with pytest.raises(ValueError, match="non-empty, trimmed string"):
        project.crs = "\n"
    with pytest.raises(ValueError, match="non-empty, trimmed string"):
        project.display_name = " Authored title "
    with pytest.raises(ValueError, match="non-empty, trimmed string"):
        project.crs = " Local CRS "
    with pytest.raises(ValueError, match="non-empty, trimmed string"):
        petekio.GeoData(unit=" m ")


def test_project_template_library_snapshots_and_mutates_explicitly(tmp_path, monkeypatch):
    project = petekio.Project(petekio.GeoData(unit="m"))
    source = _correlation_template_dict("qc/default", note={"threshold": 0.2})
    bound = project.templates.add(source, tags=["qc"])
    source["note"]["threshold"] = 999

    assert isinstance(bound, petekio.BoundTemplate)
    assert (bound.name, bound.kind, bound.schema_version) == (
        "qc/default",
        "CorrelationTemplate",
        1,
    )
    assert bound.to_dict()["note"] == {"threshold": 0.2}
    assert project.templates == ["qc/"]
    assert project.templates.qc.default.name == "qc/default"
    assert project.templates["qc/default"].to_dict() == bound.to_dict()

    with pytest.raises(ValueError, match="already exists"):
        project.templates.add(_correlation_template_dict("qc/default"))
    with pytest.raises(KeyError):
        project.templates.replace(_correlation_template_dict("missing"))

    replaced = project.templates.replace(
        _correlation_template_dict("qc/default", note={"threshold": 0.3})
    )
    assert replaced.to_dict()["note"]["threshold"] == 0.3
    renamed = project.templates.rename("qc/default", "production/reservoir")
    assert renamed.name == renamed.to_dict()["name"] == "production/reservoir"
    assert project.templates.production.reservoir.name == "production/reservoir"

    path = tmp_path / "templates.pproj"
    project.save(path)
    info = petekio.GeoData.inspect(str(path))
    assert ("asset", "@asset/templates/production/reservoir") in info["elements"]
    reopened = petekio.Project.load(path)
    assert reopened.templates.production.reservoir.to_dict() == renamed.to_dict()
    assert reopened.inventory()["templates"] == ["production/reservoir"]

    real_import = builtins.__import__

    def without_petektools(name, *args, **kwargs):
        if name == "petektools" or name.startswith("petektools."):
            raise ImportError("blocked for optional-dependency test")
        return real_import(name, *args, **kwargs)

    monkeypatch.setattr(builtins, "__import__", without_petektools)
    # Persistence/listing remains provider-free; only materialization is loud.
    assert reopened.templates.production.reservoir.name == "production/reservoir"
    with pytest.raises(ImportError, match="Install or upgrade"):
        reopened.templates.production.reservoir.materialize()

    reopened.templates.delete("production/reservoir")
    assert reopened.templates == []
    with pytest.raises(KeyError):
        reopened.templates.delete("production/reservoir")


@pytest.mark.parametrize(
    "name",
    ["../escape", "folder/../escape", "/absolute", "folder//name", "folder\\name"],
)
def test_project_template_names_reject_traversal(name):
    project = petekio.Project(petekio.GeoData(unit="m"))
    with pytest.raises((TypeError, ValueError)):
        project.templates.add(_correlation_template_dict(name))


def test_generic_unknown_asset_round_trips_without_provider(tmp_path):
    geo = petekio.GeoData(unit="m")
    envelope = json.dumps(
        {
            "asset_type": "future-value",
            "codec": "application/octet-stream",
            "future": {"untouched": True},
            "provider": "example.Future",
            "schema_version": 42,
        },
        sort_keys=True,
        separators=(",", ":"),
    ).encode()
    payload = b"\x00\xfffuture\x00"
    geo.add_asset(
        "@asset/future/example",
        "future-value",
        envelope,
        ["keep"],
        1,
        payload,
    )
    first = tmp_path / "first.pproj"
    second = tmp_path / "second.pproj"
    geo.save(str(first))
    reopened = petekio.GeoData.open(str(first))
    asset = reopened.asset("@asset/future/example")
    assert asset["envelope"] == envelope
    assert asset["bytes"] == payload
    reopened.rename_asset("@asset/future/example", "@asset/future/renamed")
    reopened.save(str(second))
    twice = petekio.GeoData.open(str(second)).asset("@asset/future/renamed")
    assert twice["envelope"] == envelope
    assert twice["bytes"] == payload


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
        root / "Surfaces" / "EarthVision_grid" / "Top Dome.EarthVisionGrid",
        """# Type: scattered data
# Field: 1 x
# Field: 2 y
# Field: 3 z meters
# Field: 4 column
# Field: 5 row
# Grid_size: 3 x 2
# Null_value: 1.0e30
# End:
100.0 200.0 -50.0 1 1
110.0 200.0 -51.0 2 1
120.0 200.0 1.0e30 3 1
100.0 210.0 -52.0 1 2
110.0 210.0 -53.0 2 2
120.0 210.0 -54.0 3 2
""",
    )
    _write(
        root / "Surfaces" / "IrapClassic_points" / "Top Dome.IrapClassicPoints",
        """100.0 200.0 -50.0
110.0 200.0 -51.0
100.0 210.0 -52.0
110.0 210.0 -53.0
120.0 210.0 -54.0
""",
    )

    project = petekio.Project.import_data(root)
    pts = project.points["Surfaces/IrapClassic_points/Top Dome"]
    surface = project.surfaces["Surfaces/EarthVision_grid/Top Dome"]

    assert isinstance(surface, petekio.StructuredMeshSurface)
    assert (surface.ncol, surface.nrow) == (3, 2)
    assert surface.node_xy(2, 0) == (120.0, 200.0)
    assert math.isnan(surface.z(2, 0))
    assert pts.attr("column") == [1.0, 2.0, 1.0, 2.0, 3.0]
    assert pts.attr("row") == [1.0, 1.0, 2.0, 2.0, 2.0]
    assert project.points.Surfaces.IrapClassic_points.top_dome.attr("row") == [
        1.0,
        1.0,
        2.0,
        2.0,
        2.0,
    ]
    geom = pts.infer_geometry(tolerance=1e-3, edge="convex_hull")
    assert geom.ncol == 3
    assert geom.nrow == 2

    saved = tmp_path / "field.pproj"
    project.save(saved)
    reopened = petekio.Project.load(saved)
    restored = reopened.surfaces["Surfaces/EarthVision_grid/Top Dome"]
    assert isinstance(restored, petekio.StructuredMeshSurface)
    assert math.isnan(restored.z(2, 0))


def test_ambiguous_earthvision_stems_do_not_guess_irap_topology(tmp_path):
    root = tmp_path / "Data"
    grid = """# Type: scattered data
# Grid_size: 2 x 2
# End:
0 0 10 1 1
10 0 11 2 1
0 10 12 1 2
10 10 13 2 2
"""
    _write(root / "A" / "Top.EarthVisionGrid", grid)
    _write(root / "B" / "Top.EarthVisionGrid", grid)
    _write(root / "Points" / "Top.IrapClassicPoints", "0 0 10\n10 0 11\n")

    project = petekio.Project.import_data(root)

    assert project.surfaces.all_names() == ["A/Top", "B/Top"]
    assert all(
        isinstance(project.surface(name), petekio.StructuredMeshSurface)
        for name in project.surfaces.all_names()
    )
    points = project.points["Points/Top"]
    assert "column" not in points.attr_names()
    assert "row" not in points.attr_names()


def _top_dome_tree(tmp_path: Path) -> Path:
    root = tmp_path / "Data"
    _write(
        root / "Surfaces" / "EarthVision_grid" / "Top Dome.EarthVisionGrid",
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
        root / "Surfaces" / "IrapClassic_points" / "Top Dome.IrapClassicPoints",
        """100.0 200.0 -50.0
110.0 200.0 -51.0
100.0 210.0 -52.0
110.0 210.0 -53.0
120.0 210.0 -54.0
""",
    )
    return root


def test_project_lookup_records_dataset_name(tmp_path):
    root = _top_dome_tree(tmp_path)
    _write(root / "Surfaces" / "Top reservoir.irap", _irap())

    project = petekio.Project.import_data(root)

    pts = project.points.Surfaces.IrapClassic_points["Top Dome"]
    assert pts.name == "Top Dome"
    # The full-path lookup resolves to the same dataset leaf name.
    assert project.points["Surfaces/IrapClassic_points/Top Dome"].name == "Top Dome"
    assert project.surfaces["Top reservoir"].name == "Top reservoir"


def test_dataset_name_propagates_to_derived_objects(tmp_path):
    project = petekio.Project.import_data(_top_dome_tree(tmp_path))
    pts = project.points.Surfaces.IrapClassic_points["Top Dome"]

    geom = pts.infer_geometry(tolerance=1e-3)
    assert geom.name == "Top Dome geometry"

    surf = pts.to_surface(geom)
    assert surf.name == "Top Dome"
    assert surf.geometry.name == "Top Dome geometry"
    assert pts.to_surface().name == "Top Dome"  # geometry inferred internally

    structured = pts.to_structured_surface(tolerance=1e-3)
    assert structured.name == "Top Dome"
    assert structured.to_tri_surface().name == "Top Dome"
    assert structured.to_points().name == "Top Dome"

    labelled, report = pts.detect_topology()
    assert report.verified
    assert labelled.name == "Top Dome"


def test_project_point_geometry_shells_propagate_name_and_kind(tmp_path):
    root = tmp_path / "Data"
    curved_rows = ["x,y,z,column,row"]
    for j in range(5):
        for i in range(5):
            curved_rows.append(
                f"{10 * i * (1 + 0.08 * i)},{10 * j * (1 + 0.05 * j)},1,{i + 1},{j + 1}"
            )
    _write(root / "Points" / "Curved.csv", "\n".join(curved_rows) + "\n")

    fault_rows = ["x,y,z"]
    for j in range(6):
        for i in range(4):
            fault_rows.append(f"{50 * i},{50 * j},-1")
        for i in range(6, 10):
            fault_rows.append(f"{50 * i + 20},{50 * j + 25},-2")
    _write(root / "Points" / "Fault.csv", "\n".join(fault_rows) + "\n")

    project = petekio.Project.import_data(root)
    curved = project.points["Curved"]
    with pytest.warns(UserWarning, match="StructuredShell geometry"):
        structured = curved.infer_geometry()
    assert structured.kind == "structured_shell"
    assert structured.name == "Curved geometry"
    assert structured.to_mesh_shell().name == "Curved geometry"
    assert curved.to_structured_surface().shell.name == "Curved geometry"

    fault = project.points["Fault"]
    with pytest.warns(UserWarning, match="MeshShell fallback"):
        mesh = fault.infer_geometry(max_bridge=None)
    assert mesh.kind == "mesh_shell"
    assert mesh.name == "Fault geometry"
    assert fault.to_tri_surface().shell.name == "Fault geometry"


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


def test_project_replace_surface_is_explicit_cow_and_persists_all_levels(tmp_path):
    root = tmp_path / "Data"
    _write(root / "Surfaces" / "Top reservoir.irap", _irap())
    _write(root / "Surfaces" / "ZZ Base reservoir.irap", _irap())
    project = petekio.Project.import_data(root)
    original_order = project.inventory()["surfaces"]
    surface = project.surface("Top reservoir")
    values = surface * 0.0 + 1.0
    metadata = {
        "id": "facies",
        "label": "Facies",
        "kind": "categorical",
        "units": None,
        "codes": {"1": {"label": "Sand", "color": "#EDA100"}},
    }
    surface.set_attr("facies", values, metadata=metadata)
    assert project.surface("Top reservoir").attr_names() == []

    project.replace_surface("Top reservoir", surface)
    assert project.surface("Top reservoir").attr_metadata("facies") == metadata

    structured = project.surface("Top reservoir").to_structured_mesh()
    project.replace_surface("Top reservoir", structured)
    assert project.surface("Top reservoir").kind == "structured_mesh"

    tri = project.surface("Top reservoir").to_tri_surface()
    project.replace_surface("Top reservoir", tri)
    assert project.surface("Top reservoir").kind == "tri_surface"

    wrong = petekio.Surface.constant(
        petekio.GridGeometry(0.0, 0.0, 1.0, 1.0, 2, 2), 1.0
    )
    with pytest.raises(ValueError, match="geometry/topology differs"):
        project.replace_surface("Top reservoir", wrong)

    pproj = tmp_path / "replaced.pproj"
    project.save(pproj)
    reopened = petekio.Project.load(pproj)
    assert reopened.inventory()["surfaces"] == original_order
    persisted = reopened.surface("Top reservoir")
    assert persisted.kind == "tri_surface"
    assert persisted.attr_metadata("facies") == metadata


@pytest.mark.parametrize("ncol,nrow", [(1, 1), (1, 4), (4, 1)])
def test_degenerate_surface_replacement_and_persistence(tmp_path, ncol, nrow):
    source = petekio.Surface.constant(
        petekio.GridGeometry(
            100.0, 200.0, 10.0, 20.0, ncol, nrow, rotation_deg=15.0
        ),
        -1800.0,
    )
    irap = tmp_path / f"degenerate_{ncol}x{nrow}.irap"
    source.save_irap_classic(str(irap))

    geo = petekio.GeoData(unit="m")
    geo.load_surface("top", str(irap))
    imported = tmp_path / f"imported_{ncol}x{nrow}.pproj"
    geo.save(str(imported))
    project = petekio.Project.load(imported)
    project.replace_surface("top", project.surface("top"))

    structured = project.surface("top").to_structured_mesh()
    project.replace_surface("top", structured)
    project.replace_surface("top", project.surface("top"))

    pproj = tmp_path / f"degenerate_{ncol}x{nrow}.pproj"
    project.save(pproj)
    reopened = petekio.Project.load(pproj)
    persisted = reopened.surface("top")
    assert persisted.kind == "structured_mesh"
    assert (persisted.ncol, persisted.nrow) == (ncol, nrow)
    reopened.replace_surface("top", persisted)


def test_surface_replacement_preserves_structured_boundary(tmp_path):
    geometry = petekio.GridGeometry(0.0, 0.0, 1.0, 1.0, 4, 4)
    irap = tmp_path / "boundary.irap"
    petekio.Surface.constant(geometry, 100.0).save_irap_classic(str(irap))
    geo = petekio.GeoData(unit="m")
    geo.load_surface("top", str(irap))
    imported = tmp_path / "boundary-imported.pproj"
    geo.save(str(imported))
    project = petekio.Project.load(imported)

    x, y, z, column, row = [], [], [], [], []
    for j in range(4):
        for i in range(4):
            x.append(float(i))
            y.append(float(j))
            z.append(math.nan if i > 1 and j > 1 else 100.0 + i + j)
            column.append(i + 1)
            row.append(j + 1)
    points = petekio.PointSet.from_xyz(x, y, z)
    points.column = column
    points.row = row
    occupied = points.to_structured_surface(edge="occupied")
    full_rect = points.to_structured_surface(edge="full_rect")
    assert occupied.edge.area() < full_rect.edge.area()

    with pytest.raises(ValueError, match="geometry/topology differs"):
        project.replace_surface("top", occupied)
    project.replace_surface("top", full_rect)
    with pytest.raises(ValueError, match="geometry/topology differs"):
        project.replace_surface("top", occupied)

    project.replace_surface("top", full_rect.to_tri_surface())
    persisted_path = tmp_path / "boundary-persisted.pproj"
    project.save(persisted_path)
    persisted = petekio.Project.load(persisted_path).surface("top")
    assert persisted.kind == "tri_surface"
    assert math.isclose(persisted.edge.area(), full_rect.edge.area())


def test_project_folder_navigation_and_object_management(tmp_path):
    root = tmp_path / "Data"
    _write(root / "Surfaces" / "Top reservoir.irap", _irap())
    _write(root / "Points" / "samples.csv", "x,y,z,poro\n1,2,-3,0.2\n4,5,-6,0.3\n")
    _write(root / "Polygons" / "ModelEdge.geojson", _polygon_geojson())
    _write(root / "Wells" / "99_9-1_A_CompLogs.las", _las())
    _write(root / "WellTops.tops", _petrel_tops())

    project = petekio.Project.import_data(root)

    project.rename_surface("Top reservoir", "structure/top dome")
    assert project.surfaces == ["structure/"]
    assert project.structures == ["structure/"]
    assert project.surfaces.structure == ["top dome"]
    assert project.surfaces.structure.top_dome.stats().count == 4
    assert project.surfaces.top_dome.stats().count == 4
    assert project.inventory()["surfaces"] == ["structure/top dome"]

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
    assert reopened.surfaces.top_dome.stats().count == 4
    assert reopened.points.data.samples.stats("poro").count == 2

    reopened.delete_surface("top dome")
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
