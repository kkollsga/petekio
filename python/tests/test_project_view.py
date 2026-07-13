"""Lazy project workspace provider and exact petekTools schema-v1 seam."""

from __future__ import annotations

import base64
import builtins
import struct
import time

import pytest

import petekio
from petekio._project_view import ProjectViewProvider
from petekio._project_view_curves import select_auto_curves


viewer = pytest.importorskip("petektools.viewer")


class _Names:
    def __init__(self, names=()):
        self._names = list(names)

    def all_names(self):
        return list(self._names)


class _Wells(_Names):
    def names(self):
        return list(self._names)


class _EmptyTops(_Names):
    def __getitem__(self, name):
        raise KeyError(name)


class _TemplateSpy(_Names):
    def __init__(self, names=()):
        super().__init__(names)
        self.loads = 0

    def __getitem__(self, name):
        self.loads += 1
        return object()


class _Assets:
    def __init__(self, names=()):
        self._names = list(names)

    def asset_names(self):
        return list(self._names)


class _BoreSpy:
    def __init__(self, mnemonics=("PHIE",)):
        self.log_calls = 0
        self.mnemonic_calls = 0
        self._mnemonics = list(mnemonics)

    def mnemonics(self):
        self.mnemonic_calls += 1
        return list(self._mnemonics)

    def _view_raw(self, curves=None):
        self.log_calls += 1
        raise AssertionError("log gathering is not catalog metadata")


class _WellSpy:
    head = (0.0, 0.0)

    def __init__(self, bores=("",)):
        self._bores = list(bores)
        self._items = {name: _BoreSpy() for name in self._bores}

    def bores(self):
        return list(self._bores)

    def sidetrack(self, name):
        return self._items.get(name)


class _Hit:
    def __init__(self, md, xyz):
        self.md = md
        self.xyz = xyz


class _OverlayBore(_BoreSpy):
    def __init__(self, hits):
        super().__init__(mnemonics=())
        self.hits = list(hits)
        self.intersection_calls = 0

    def md_range(self):
        return (0.0, 100.0)

    def xyz(self, md):
        return (1000.0 + md, 2000.0, -md)

    def intersections(self, surface):
        self.intersection_calls += 1
        return list(self.hits)


class _OverlayWell(_WellSpy):
    def __init__(self, hits):
        self._bores = [""]
        self._items = {"": _OverlayBore(hits)}


class _SurfaceSpy:
    kind = "surface"

    def __init__(self, name, attrs=("thickness",)):
        self.name = name.rsplit("/", 1)[-1]
        self.attrs = list(attrs)
        self.calls = []

    def attr_names(self):
        return list(self.attrs)

    def value_layer(self, attr=None, stride=None):
        self.calls.append((attr, stride))
        offset = 10.0 if attr == "thickness" else 0.0
        return {
            "kind": "trimesh",
            "name": "values" if attr is None else attr,
            "nodes": [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]],
            "triangles": [[0, 1, 2]],
            "values": [offset + 1.0, offset + 2.0, offset + 3.0],
            "range": [offset + 1.0, offset + 3.0],
        }


class _NativeSurfaceSpy(_SurfaceSpy):
    ncol = 2
    nrow = 2

    def __init__(self, name):
        super().__init__(name, attrs=())
        self.native_calls = []

    def _view_regular_grid(self, attr=None, stride=1):
        self.native_calls.append((attr, stride))
        values = struct.pack("<4f", 1.0, 2.0, 3.0, 4.0)
        mask = bytes((1, 1, 1, 1))
        return {
            "name": "values",
            "dimensions": [2, 2],
            "origin": [0.0, 0.0],
            "step_i": [1.0, 0.0],
            "step_j": [0.0, 1.0],
            "elevations": values,
            "values": values,
            "elevation_mask": mask,
            "value_mask": mask,
            "elevation_range": [1.0, 4.0],
            "range": [1.0, 4.0],
            "triangle_count": 2,
            "stride": stride,
        }

    def value_layer(self, attr=None, stride=None):
        raise AssertionError("native compact resources must not call value_layer()")


class _FakeProject:
    def __init__(self, surfaces, *, assets=(), wells=None, templates=None):
        self._surfaces = dict(surfaces)
        self._wells = dict(wells or {})
        self.surfaces = _Names(self._surfaces)
        self.points = _Names()
        self.polygons = _Names()
        self.wells = _Wells(self._wells)
        self.well_tops = _EmptyTops()
        self.tops = _EmptyTops()
        self.templates = templates or _EmptyTops()
        self.geodata = _Assets(assets)

    def surface(self, name):
        return self._surfaces.get(name)

    def point_set(self, name):
        return None

    def polygon_set(self, name):
        return None

    def well(self, name):
        return self._wells.get(name)


def _walk(nodes):
    for node in nodes:
        if "children" in node:
            yield from _walk(node["children"])
        else:
            yield node


def test_catalog_is_metadata_only_and_ids_use_full_encoded_paths():
    a = _SurfaceSpy("Interpretation/A/Top A")
    b = _SurfaceSpy("Interpretation/B/Top A")
    project = _FakeProject({"Interpretation/A/Top A": a, "Interpretation/B/Top A": b})

    provider = ProjectViewProvider(project, lod=False)
    leaves = list(_walk(provider.view_catalog()))

    assert [leaf["id"] for leaf in leaves] == [
        "surface:Interpretation/A/Top%20A",
        "surface:Interpretation/B/Top%20A",
    ]
    assert leaves[0]["views"]["map"] == {
        "lanes": [
            {"id": "depth", "label": "Depth"},
            {"id": "thickness", "label": "Thickness"},
        ],
        "active_lane": "depth",
    }
    assert "tiers" not in leaves[0]["views"]["scene3d"]
    assert leaves[0]["visible"] == {"map": True, "scene3d": True}
    assert leaves[1]["visible"] == {"map": False, "scene3d": False}
    assert a.calls == b.calls == []


def test_bore_ids_are_typed_and_log_gathering_is_never_catalog_work():
    well = _WellSpy(("", "ST 2"))
    provider = ProjectViewProvider(
        _FakeProject({}, wells={"A/1": well}),
        logs=petekio.ViewSpec(curves=("PHIE",)),
    )
    leaves = list(_walk(provider.view_catalog()))

    assert [leaf["id"] for leaf in leaves] == [
        "well:A%2F1/bore:%00",
        "well:A%2F1/bore:ST%202",
    ]
    assert all("wells" in leaf["views"] for leaf in leaves)
    assert all(leaf["visible"]["wells"] is False for leaf in leaves)
    assert all(bore.log_calls == 0 for bore in well._items.values())
    assert all(bore.mnemonic_calls == 1 for bore in well._items.values())


def test_automatic_curve_priorities_are_small_and_cross_bore_consistent():
    selected = select_auto_curves(
        {
            "a": ["MD", "CALI", "GR", "FACIES", "VSH", "PHIE", "SW", "PERM", "RDEP"],
            "b": ["DEPTH", "DT", "GAMMA", "VCL", "NPHI", "SWT", "K", "RT"],
        }
    )
    assert selected == {
        "a": ("GR", "VSH", "PHIE", "SW", "PERM", "RDEP"),
        "b": ("GAMMA", "VCL", "NPHI", "SWT", "K", "RT"),
    }


def test_automatic_curve_fallback_skips_coordinates_and_discrete_curves():
    selected = select_auto_curves(
        {
            "a": ["MD", "CALI", "DT", "RHOB", "FACIES", "MISC"],
            "b": ["DEPTH", "CALI", "DT", "NPHI", "NETFLAG"],
        }
    )
    assert selected["a"] == ("CALI", "DT", "RHOB")
    assert selected["b"] == ("NPHI", "CALI", "DT")
    assert all(len(curves) <= 6 for curves in selected.values())


def test_explicit_template_payload_is_still_lazy_during_catalog_build():
    templates = _TemplateSpy(("qc/reservoir",))
    provider = ProjectViewProvider(
        _FakeProject({}, wells={"A": _WellSpy()}, templates=templates),
        logs=petekio.ViewSpec(curves=("PHIE",)),
        template="qc/reservoir",
    )

    assert list(_walk(provider.view_catalog()))
    assert templates.loads == 0


def test_workspace_cache_materializes_only_requested_surface_lane():
    surface = _SurfaceSpy("Top A")
    provider = ProjectViewProvider(
        _FakeProject({"Top A": surface}), lod=False
    )
    session = viewer.view(provider, serve=False)
    item_id = "surface:Top%20A"

    primary = session.resource(item_id, "map", "depth")
    assert primary["lane"] == "depth"
    assert surface.calls == [(None, None)]
    assert session.resource(item_id, "map", "depth") == primary
    assert surface.calls == [(None, None)]

    thickness = session.resource(item_id, "map", "thickness")
    assert thickness["payload"]["map"]["fills"][0]["name"] == "thickness"
    assert surface.calls == [(None, None), ("thickness", None)]


def test_regular_structured_and_tri_surface_resources_share_the_lane_seam(tmp_path):
    path = tmp_path / "tiny.irap"
    path.write_text(
        "-996 2 10 10\n0 10 0 10\n2 0 0 0\n0 0 0 0 0 0 0\n1 2 3 4\n"
    )
    regular = petekio.Surface.load_irap_classic(str(path))
    surfaces = [regular, regular.to_structured_mesh(), regular.to_tri_surface()]

    for index, surface in enumerate(surfaces):
        name = f"Variant/{index}"
        provider = ProjectViewProvider(_FakeProject({name: surface}), lod=False)
        item_id = f"surface:Variant/{index}"
        payload = provider.view_resource(item_id=item_id, view="map", lane="depth")
        assert payload["map"]["fills"][0]["name"] == "values"
        fill = payload["map"]["fills"][0]
        if index == 0:
            assert "regular_grid" in fill
            assert "nodes" not in fill and "triangles" not in fill
        else:
            # These types retain the general petekTools value_layer fallback;
            # the renderer may itself compact an affine shell afterwards.
            assert not hasattr(surface, "_view_regular_grid")


def _block_values(payload, marker, code):
    block = payload["map"]["blocks"][marker["__block__"]]
    raw = base64.b64decode(block["data"])
    return struct.unpack(f"<{len(raw) // struct.calcsize(code)}{code}", raw)


def _direct_block_values(block, code):
    raw = base64.b64decode(block["data"])
    return struct.unpack(f"<{len(raw) // struct.calcsize(code)}{code}", raw)


def test_regular_surface_map_is_native_affine_row_major_and_nan_exact():
    surface = petekio.Surface.load_irap_classic("tests/fixtures/simple.irap")
    surface.thickness = petekio.Surface.constant(surface.geometry, 7.0)
    provider = ProjectViewProvider(_FakeProject({"Rotated/Top": surface}))
    item_id = "surface:Rotated/Top"

    depth = provider.view_resource(item_id=item_id, view="map", lane="depth")
    fill = depth["map"]["fills"][0]
    grid = fill["regular_grid"]
    assert grid["dimensions"] == [3, 4]
    assert grid["origin"] == pytest.approx([1000.0, 2000.0])
    assert grid["step_i"] == pytest.approx([43.3012701892, 25.0])
    assert grid["step_j"] == pytest.approx([-12.5, 21.6506350946])
    values = _block_values(depth, grid["values"], "f")
    assert values[:5] == pytest.approx((1000.0, 1010.0, 1020.0, 1005.0, 1015.0))
    assert values[5] != values[5]
    assert _block_values(depth, grid["mask"], "B")[5] == 0
    assert "nodes" not in fill and "triangles" not in fill

    thickness = provider.view_resource(item_id=item_id, view="map", lane="thickness")
    attr_grid = thickness["map"]["fills"][0]["regular_grid"]
    assert set(_block_values(thickness, attr_grid["values"], "f")) == {7.0}


def test_regular_surface_scene_has_progressive_compact_detail():
    geom = petekio.GridGeometry(
        1000.0, 2000.0, 25.0, 30.0, 750, 750, rotation_deg=23.0, yflip=True
    )
    surface = petekio.Surface.constant(geom, -2500.0)
    provider = ProjectViewProvider(_FakeProject({"Large": surface}))
    leaf = next(_walk(provider.view_catalog()))
    assert leaf["views"]["scene3d"]["tiers"] == [
        {"id": "preview", "label": "Preview"},
        {"id": "full", "label": "Full detail"},
    ]

    started = time.perf_counter()
    preview = provider.view_resource(
        item_id="surface:Large", view="scene3d", lane="depth", detail="preview"
    )
    preview_seconds = time.perf_counter() - started
    mesh = preview["scene3d"]["meshes"][0]
    assert "regular_surface" in mesh
    assert "nodes" not in mesh and "triangles" not in mesh
    assert preview["scene3d"]["detail"] == "preview"
    assert preview["scene3d"]["preview_stride"] > 1
    assert preview_seconds < 0.25

    started = time.perf_counter()
    full = provider.view_resource(
        item_id="surface:Large", view="scene3d", lane="depth", detail="full"
    )
    full_seconds = time.perf_counter() - started
    full_grid = full["scene3d"]["meshes"][0]["regular_surface"]
    preview_grid = mesh["regular_surface"]
    assert full_grid["dimensions"] == [750, 750]
    # The coarse preview spans the exact full footprint even though 749 is not
    # divisible by its stride; swapping detail must not change camera framing.
    for axis in (0, 1):
        preview_end = (
            preview_grid["origin"][axis]
            + (preview_grid["dimensions"][0] - 1) * preview_grid["step_i"][axis]
            + (preview_grid["dimensions"][1] - 1) * preview_grid["step_j"][axis]
        )
        full_end = (
            full_grid["origin"][axis]
            + 749 * full_grid["step_i"][axis]
            + 749 * full_grid["step_j"][axis]
        )
        assert preview_end == pytest.approx(full_end)
    assert full_seconds < 0.5

    started = time.perf_counter()
    map_payload = provider.view_resource(
        item_id="surface:Large", view="map", lane="depth"
    )
    map_seconds = time.perf_counter() - started
    assert "regular_grid" in map_payload["map"]["fills"][0]
    assert map_payload["map"]["fills"][0]["regular_grid"]["step_j"][1] < 0
    assert map_seconds < 0.5


def test_native_regular_resources_never_call_full_mesh_duck():
    surface = _NativeSurfaceSpy("Native")
    provider = ProjectViewProvider(_FakeProject({"Native": surface}))

    provider.view_resource(item_id="surface:Native", view="map", lane="depth")
    provider.view_resource(
        item_id="surface:Native", view="scene3d", lane="depth", detail="preview"
    )
    assert surface.native_calls == [(None, 1), (None, 1)]


def test_scene_attribute_nulls_do_not_cut_finite_depth_geometry():
    attribute = petekio.Surface.load_irap_classic("tests/fixtures/simple.irap")
    primary = petekio.Surface.constant(attribute.geometry, -2500.0)
    primary.thickness = attribute
    provider = ProjectViewProvider(_FakeProject({"Top": primary}))

    payload = provider.view_resource(
        item_id="surface:Top", view="scene3d", lane="thickness", detail="full"
    )
    regular = payload["scene3d"]["meshes"][0]["regular_surface"]
    mask = _direct_block_values(regular["mask"], "B")
    values = _direct_block_values(regular["values"], "f")
    elevations = _direct_block_values(regular["elevations"], "f")
    assert mask[5] == 1
    assert values[5] != values[5]
    assert elevations[5] == -2500.0


def test_surface_map_overlay_ends_at_first_exact_md_hit_and_is_cached():
    first = _Hit(40.25, (1040.25, 2000.0, -40.25))
    second = _Hit(80.5, (1080.5, 2000.0, -80.5))
    well = _OverlayWell((second, first))
    surface = _SurfaceSpy("Top")
    provider = ProjectViewProvider(
        _FakeProject({"Top": surface}, wells={"A": well}), lod=False
    )

    depth = provider.view_resource(item_id="surface:Top", view="map", lane="depth")
    overlay = depth["map"]["well_overlays"][0]
    assert overlay["context_item_id"] == "surface:Top"
    assert overlay["well_item_id"] == "well:A/bore:%00"
    assert overlay["status"] == "hit"
    assert overlay["intersection"] == {
        "md": 40.25,
        "xyz": [1040.25, 2000.0, -40.25],
    }
    assert overlay["trajectory"][-1] == [1040.25, 2000.0, -40.25]
    assert "first MD-ordered hit" in overlay["message"]

    provider.view_resource(item_id="surface:Top", view="map", lane="thickness")
    assert well._items[""].intersection_calls == 1


def test_selection_folders_properties_visibility_and_duplicate_leaf_guidance():
    surfaces = {
        "A/Top": _SurfaceSpy("A/Top"),
        "A/Base": _SurfaceSpy("A/Base"),
        "B/Top": _SurfaceSpy("B/Top"),
    }
    provider = ProjectViewProvider(
        _FakeProject(surfaces),
        selection={"surfaces": ["A/"]},
        visible={"map": ["A/Base"]},
        property="thickness",
        lod=False,
    )
    leaves = list(_walk(provider.view_catalog()))
    assert [leaf["id"] for leaf in leaves] == ["surface:A/Top", "surface:A/Base"]
    assert [leaf["visible"]["map"] for leaf in leaves] == [False, True]
    assert all(leaf["views"]["map"]["active_lane"] == "thickness" for leaf in leaves)

    with pytest.raises(ValueError, match="ambiguous.*canonical full path"):
        ProjectViewProvider(
            _FakeProject(surfaces), selection={"surfaces": ["Top"]}
        )


def test_unknown_assets_are_disabled_and_preserved_in_catalog():
    project = _FakeProject(
        {"Top": _SurfaceSpy("Top")}, assets=["@asset/future/example"]
    )
    provider = ProjectViewProvider(project)
    unknown = next(leaf for leaf in _walk(provider.view_catalog()) if leaf["role"] == "asset")

    assert unknown["id"] == "asset:%40asset/future/example"
    assert unknown["views"] == {}
    assert unknown["disabled"] is True
    assert "opaque asset" in unknown["reason"]
    assert any(d["code"] == "unsupported_asset" for d in provider.diagnostics)

    normalized = viewer.view(provider, serve=False).tree()
    normalized_unknown = next(leaf for leaf in _walk(normalized) if leaf["role"] == "asset")
    assert normalized_unknown["disabled"] is True


def test_refresh_reports_deleted_explicit_selection_and_updates_default_catalog():
    project = _FakeProject({"A/Top": _SurfaceSpy("A/Top"), "B/Base": _SurfaceSpy("B/Base")})
    provider = ProjectViewProvider(
        project, selection={"surfaces": ["A/Top", "B/Base"]}, lod=False
    )
    project._surfaces.pop("A/Top")
    project.surfaces._names.remove("A/Top")

    with pytest.raises(KeyError, match="renamed or deleted"):
        provider.view_resource(item_id="surface:A/Top", view="map", lane="depth")
    provider.refresh()
    assert [leaf["id"] for leaf in _walk(provider.view_catalog())] == ["surface:B/Base"]
    assert any(d["code"] == "selection_missing" for d in provider.diagnostics)


def test_project_view_inspection_needs_no_optional_viewer_import(monkeypatch):
    project = petekio.Project(petekio.GeoData(unit="m"))
    real_import = builtins.__import__

    def blocked(name, *args, **kwargs):
        if name == "petektools" or name.startswith("petektools."):
            raise ImportError("blocked")
        return real_import(name, *args, **kwargs)

    monkeypatch.setattr(builtins, "__import__", blocked)
    session = project.view(settings=petekio.ViewSettings(serve=False))
    assert isinstance(session, petekio.ProjectViewSession)
    assert session.tree() == []
    with pytest.raises(ImportError, match="optional petekTools workspace"):
        session.manifest()


def test_static_visible_embeds_active_lane_selected_embeds_all(tmp_path):
    visible_spy = _SurfaceSpy("Top")
    visible_provider = ProjectViewProvider(
        _FakeProject({"Top": visible_spy}), property="thickness", lod=False
    )
    viewer.view(visible_provider, serve=False).save(tmp_path / "visible.html")
    assert visible_spy.calls == [("thickness", None), ("thickness", None)]

    selected_spy = _SurfaceSpy("Top")
    selected_provider = ProjectViewProvider(
        _FakeProject({"Top": selected_spy}), lod=False
    )
    viewer.view(selected_provider, serve=False).save(
        tmp_path / "selected.html", include="selected"
    )
    assert selected_spy.calls == [
        (None, None),
        ("thickness", None),
        (None, None),
        ("thickness", None),
    ]


def test_project_and_toolkit_notebook_entry_points_are_equivalent():
    project = petekio.Project(petekio.GeoData(unit="m"))
    # An unknown asset supplies a disabled leaf, allowing a workspace snapshot
    # without materializing any domain data.
    project.geodata.add_asset(
        "@asset/future/example",
        "future",
        b'{"asset_type":"future","codec":"bytes","provider":"example.Future","schema_version":1}',
        [],
        1,
        b"opaque",
    )
    own = project.view(settings=petekio.ViewSettings(serve=False))
    generic = viewer.view(project, serve=False)
    own_ids = [leaf["id"] for leaf in _walk(own.tree())]
    generic_ids = [leaf["id"] for leaf in _walk(generic.tree())]
    assert own_ids == generic_ids == ["asset:%40asset/future/example"]


def test_generic_project_provider_forwards_progressive_detail(tmp_path):
    path = tmp_path / "tiny.irap"
    path.write_text(
        "-996 2 10 10\n0 10 0 10\n2 0 0 0\n0 0 0 0 0 0 0\n1 2 3 4\n"
    )
    project = petekio.Project.import_data(tmp_path)
    item_id = next(_walk(project.view_catalog()))["id"]

    resource = project.view_resource(
        item_id=item_id, view="scene3d", lane="depth", detail="preview"
    )
    assert resource["scene3d"]["detail"] == "preview"
    assert "regular_surface" in resource["scene3d"]["meshes"][0]


def test_bore_logs_are_discovered_from_metadata_and_explicit_spec_filters(tmp_path):
    las = (
        "~Version\n VERS. 2.0 :\n WRAP. NO :\n"
        "~Well\n STRT.M 100 :\n STOP.M 102 :\n STEP.M 1 :\n NULL. -999.25 :\n"
        "~Curve\n DEPT.M :\n PHIE.v/v :\n~ASCII\n"
        "100 0.2\n101 0.25\n102 0.3\n"
    )
    root = tmp_path / "Data"
    root.mkdir()
    (root / "99_1-1.las").write_text(las)
    project = petekio.Project.import_data(root)

    plain = project.view(settings=petekio.ViewSettings(serve=False))
    bore = next(leaf for leaf in _walk(plain.tree()) if leaf["role"] == "bore")
    assert bore["views"]["wells"] == {}
    assert bore["visible"]["wells"] is False
    resource = plain.resource(bore["id"], "wells")
    assert [c["mnemonic"] for c in resource["payload"]["wells_logs"]["wells"][0]["curves"]] == ["PHIE"]


def test_automatic_resource_caps_tracks_and_template_or_spec_is_authoritative(tmp_path):
    las = (
        "~Version\n VERS. 2.0 :\n WRAP. NO :\n"
        "~Well\n STRT.M 100 :\n STOP.M 101 :\n STEP.M 1 :\n NULL. -999.25 :\n"
        "~Curve\n DEPT.M :\n CALI.IN :\n GR.API :\n FACIES.NONE :\n"
        " VSH.v/v :\n PHIE.v/v :\n SW.v/v :\n PERM.MD :\n RDEP.OHMM :\n NPHI.v/v :\n"
        "~ASCII\n"
        "100 8.5 45 1 0.2 0.18 0.3 120 20 0.21\n"
        "101 8.6 50 2 0.3 0.16 0.4 100 15 0.19\n"
    )
    root = tmp_path / "Data"
    root.mkdir()
    (root / "99_2-1.las").write_text(las)
    project = petekio.Project.import_data(root)

    automatic = project.view(settings=petekio.ViewSettings(serve=False))
    bore = next(leaf for leaf in _walk(automatic.tree()) if leaf["role"] == "bore")
    resource = automatic.resource(bore["id"], "wells")
    curves = resource["payload"]["wells_logs"]["wells"][0]["curves"]
    assert [curve["mnemonic"] for curve in curves] == [
        "GR",
        "VSH",
        "PHIE",
        "SW",
        "PERM",
        "RT",
    ]
    assert curves[-1]["display_name"] == "RDEP"

    explicit = project.view(
        logs=petekio.ViewSpec(curves=("CALI", "PHIE")),
        settings=petekio.ViewSettings(serve=False),
    )
    curves = explicit.resource(bore["id"], "wells")["payload"]["wells_logs"]["wells"][0]["curves"]
    assert [curve["mnemonic"] for curve in curves] == ["CALI", "PHIE"]

    template = viewer.CorrelationTemplate("qc").add_track(
        viewer.CorrelationTrack("selected").curve("CALI").curve("GR")
    )
    templated = project.view(
        template=template,
        settings=petekio.ViewSettings(serve=False),
    )
    curves = templated.resource(bore["id"], "wells")["payload"]["wells_logs"]["wells"][0]["curves"]
    assert [curve["mnemonic"] for curve in curves] == ["CALI", "GR"]

    logs = project.view(
        logs=petekio.ViewSpec(curves=("PHIE",)),
        settings=petekio.ViewSettings(serve=False),
    )
    bore = next(leaf for leaf in _walk(logs.tree()) if leaf["role"] == "bore")
    assert bore["views"]["wells"] == {}
    resource = logs.resource(bore["id"], "wells")
    assert [c["mnemonic"] for c in resource["payload"]["wells_logs"]["wells"][0]["curves"]] == ["PHIE"]
