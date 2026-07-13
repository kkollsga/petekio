"""Lazy project workspace provider and exact petekTools schema-v1 seam."""

from __future__ import annotations

import builtins

import pytest

import petekio
from petekio._project_view import ProjectViewProvider


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

    logs = project.view(
        logs=petekio.ViewSpec(curves=("PHIE",)),
        settings=petekio.ViewSettings(serve=False),
    )
    bore = next(leaf for leaf in _walk(logs.tree()) if leaf["role"] == "bore")
    assert bore["views"]["wells"] == {}
    resource = logs.resource(bore["id"], "wells")
    assert [c["mnemonic"] for c in resource["payload"]["wells_logs"]["wells"][0]["curves"]] == ["PHIE"]
