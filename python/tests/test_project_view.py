"""Lazy project workspace provider and petekTools workspace-v2 seam."""

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
        if len(self.hits) == 1 and isinstance(self.hits[0], Exception):
            raise self.hits[0]
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

    def _view_shared_regular_grid(self, attrs, stride=1):
        self.native_calls.append((tuple(attrs), stride))
        values = struct.pack("<4f", 1.0, 2.0, 3.0, 4.0)
        return {
            "dimensions": [2, 2],
            "origin": [0.0, 0.0],
            "step_i": [1.0, 0.0],
            "step_j": [0.0, 1.0],
            "lanes": [{"values": values, "range": [1.0, 4.0]}],
            "mask": bytes((1, 1, 1, 1)),
            "triangle_count": 2,
            "stride": stride,
        }

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


class _SharedMultiSurfaceSpy(_SurfaceSpy):
    ncol = 2
    nrow = 2

    def __init__(self, *, fractional_category=False, missing_lane=False, all_nan=False):
        super().__init__("Shared", attrs=("porosity", "facies"))
        self.shared_calls = []
        self.fractional_category = fractional_category
        self.missing_lane = missing_lane
        self.all_nan = all_nan

    def attr_metadata(self, name):
        if name == "facies":
            return {
                "id": "facies",
                "label": "Facies",
                "kind": "categorical",
                "units": None,
                "codes": {"1": {"label": "Sand", "color": "#EDA100"}},
            }
        return {
            "id": "porosity",
            "label": "Porosity",
            "kind": "continuous",
            "units": "v/v",
            "codes": None,
        }

    def _view_shared_regular_grid(self, attrs, stride=1):
        self.shared_calls.append((tuple(attrs), stride))
        rows = []
        for attr in attrs:
            if attr is None:
                values = (10.0, 11.0, 12.0, 13.0)
            elif attr == "porosity":
                values = (float("nan"),) * 4 if self.all_nan else (0.1, 0.2, 0.3, 0.4)
            else:
                values = (1.5, 1.0, 1.0, 1.0) if self.fractional_category else (1.0, 1.0, 1.0, 1.0)
            finite = [value for value in values if value == value]
            rows.append(
                {
                    "values": struct.pack("<4f", *values),
                    "range": [min(finite), max(finite)] if finite else None,
                }
            )
        if self.missing_lane:
            rows.pop()
        return {
            "dimensions": [2, 2],
            "origin": [0.0, 0.0],
            "step_i": [1.0, 0.0],
            "step_j": [0.0, 1.0],
            "lanes": rows,
            "mask": bytes((1, 1, 1, 1)),
            "triangle_count": 2,
            "stride": stride,
        }

    def value_layer(self, attr=None, stride=None):
        raise AssertionError("shared resources must not materialize legacy meshes")


class _FakeProject:
    def __init__(
        self,
        surfaces,
        *,
        assets=(),
        wells=None,
        templates=None,
        display_name=None,
        crs=None,
        unit="m",
    ):
        self._surfaces = dict(surfaces)
        self._wells = dict(wells or {})
        self.display_name = display_name
        self.crs = crs
        self.unit = unit
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


def _provider_tree(provider):
    return provider.view_catalog()["tree"]


def test_catalog_is_metadata_only_and_ids_use_full_encoded_paths():
    a = _SurfaceSpy("Interpretation/A/Top A")
    b = _SurfaceSpy("Interpretation/B/Top A")
    project = _FakeProject({"Interpretation/A/Top A": a, "Interpretation/B/Top A": b})

    provider = ProjectViewProvider(project, lod=False)
    assert provider.view_catalog()["schema_version"] == 2
    assert "project" not in provider.view_catalog()
    leaves = list(_walk(_provider_tree(provider)))

    assert [leaf["id"] for leaf in leaves] == [
        "surface:Interpretation/A/Top%20A",
        "surface:Interpretation/B/Top%20A",
    ]
    assert leaves[0]["views"]["map"] == {
        "attributes": [
            {
                "id": "depth",
                "label": "Depth",
                "kind": "continuous",
                "units": "m",
                "codes": None,
            },
            {
                "id": "thickness",
                "label": "Thickness",
                "kind": "continuous",
                "units": None,
                "codes": None,
            },
        ],
        "active_attribute": "depth",
        "active_color_by": "depth",
    }
    assert "tiers" not in leaves[0]["views"]["scene3d"]
    assert leaves[0]["visible"] == {"map": True, "scene3d": True}
    assert leaves[1]["visible"] == {"map": False, "scene3d": False}
    assert a.calls == b.calls == []


def test_v2_catalog_uses_persisted_project_and_attribute_metadata(tmp_path):
    root = tmp_path / "Data"
    root.mkdir()
    (root / "Top.irap").write_text(
        "-996 2 10 10\n0 10 0 10\n2 0 0 0\n0 0 0 0 0 0 0\n1 2 3 4\n"
    )
    project = petekio.Project.import_data(
        root,
        display_name="Field model",
        settings=petekio.ImportSettings(crs="Local rotated grid", unit="m"),
    )
    surface = project.surface("Top")
    facies = surface * 0.0 + 1.0
    metadata = {
        "id": "facies",
        "label": "Depositional facies",
        "kind": "categorical",
        "units": None,
        "codes": {
            "1": {"label": "Sand", "color": "#EDA100"},
            "2": {"label": None, "color": None},
        },
    }
    surface.set_attr("facies", facies, metadata=metadata)
    project.replace_surface("Top", surface)
    path = tmp_path / "field.pproj"
    project.save(path)
    reopened = petekio.Project.load(path)

    provider = ProjectViewProvider(reopened, lod=False)
    catalog = provider.view_catalog()
    assert catalog["project"] == {
        "title": "Field model",
        "crs": "Local rotated grid",
        "unit": "m",
    }
    leaf = next(_walk(catalog["tree"]))
    descriptors = leaf["views"]["map"]["attributes"]
    assert descriptors == [
        {
            "id": "depth",
            "label": "Depth",
            "kind": "continuous",
            "units": "m",
            "codes": None,
        },
        metadata,
    ]
    assert leaf["views"]["map"]["active_attribute"] == "depth"
    assert leaf["views"]["map"]["active_color_by"] == "depth"
    assert leaf["views"]["map"]["transport"] == "shared"
    assert leaf["views"]["map"]["modes"] == ["2d", "3d"]
    assert "lanes" not in leaf["views"]["map"]

    session = reopened.view(settings=petekio.ViewSettings(serve=False))
    workspace = session.manifest()["workspace"]
    assert workspace["schema_version"] == 2
    assert workspace["title"] == "Field model"
    assert workspace["project"] == catalog["project"]
    normalized_leaf = next(_walk(workspace["tree"]))
    assert normalized_leaf["views"] == ["map"]
    assert set(normalized_leaf["resources"]) == {"map"}
    assert session.diagnostics == ()
    resource = session.resource(leaf["id"], "map", detail="full")
    assert resource["schema_version"] == 2
    assert not {"lane", "attribute", "color_by"} & set(resource)
    assert "scene3d" not in resource["payload"]
    shared = resource["payload"]["map"]["surface_grid"]
    assert [attribute["id"] for attribute in shared["attributes"]] == [
        "depth",
        "facies",
    ]
    assert shared["attributes"][1]["codes"] == metadata["codes"]

    direct = provider.view_resource(
        item_id=leaf["id"],
        view="map",
        detail="full",
    )
    assert direct["kind"] == "workspace_resource"
    with pytest.raises(ValueError, match="do not accept selectors"):
        provider.view_resource(
            item_id=leaf["id"],
            view="map",
            attribute="depth",
            color_by="facies",
        )


def test_lowercase_categorical_color_is_canonical_at_exact_tools_seam():
    geometry = petekio.GridGeometry(0.0, 0.0, 1.0, 1.0, 2, 2)
    surface = petekio.Surface.constant(geometry, -1800.0)
    surface.set_attr(
        "facies",
        petekio.Surface.constant(geometry, 1.0),
        metadata={
            "id": "facies",
            "label": "Facies",
            "kind": "categorical",
            "units": None,
            "codes": {"1": {"label": "Sand", "color": "#eda100"}},
        },
    )
    assert surface.attr_metadata("facies")["codes"]["1"]["color"] == "#EDA100"

    session = viewer.view(
        ProjectViewProvider(_FakeProject({"Top": surface})), serve=False
    )
    spec = next(_walk(session.manifest()["workspace"]["tree"]))["resources"]["map"]
    descriptor = next(item for item in spec["attributes"] if item["id"] == "facies")
    assert descriptor["codes"]["1"]["color"] == "#EDA100"
    resource = session.resource("surface:Top", "map", detail="full")
    fetched = resource["payload"]["map"]["surface_grid"]["attributes"][1]
    assert {key: fetched[key] for key in descriptor} == descriptor


def test_bore_ids_are_typed_and_log_gathering_is_never_catalog_work():
    well = _WellSpy(("", "ST 2"))
    provider = ProjectViewProvider(
        _FakeProject({}, wells={"A/1": well}),
        logs=petekio.ViewSpec(curves=("PHIE",)),
    )
    leaves = list(_walk(_provider_tree(provider)))

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

    assert list(_walk(_provider_tree(provider)))
    assert templates.loads == 0


def test_workspace_cache_materializes_only_requested_surface_lane():
    surface = _SurfaceSpy("Top A")
    provider = ProjectViewProvider(
        _FakeProject({"Top A": surface}), lod=False
    )
    session = viewer.view(provider, serve=False)
    item_id = "surface:Top%20A"

    primary = session.resource(
        item_id, "map", attribute="depth", color_by="depth"
    )
    assert primary["attribute"] == primary["color_by"] == "depth"
    assert surface.calls == [(None, None)]
    assert (
        session.resource(item_id, "map", attribute="depth", color_by="depth")
        == primary
    )
    assert surface.calls == [(None, None)]

    thickness = session.resource(
        item_id, "map", attribute="thickness", color_by="thickness"
    )
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
        if index == 0:
            payload = provider.view_resource(
                item_id=item_id, view="map", detail="full"
            )
            shared = payload["payload"]["map"]["surface_grid"]
            assert shared["attributes"][0]["id"] == "depth"
            assert "scene3d" not in payload["payload"]
        else:
            payload = provider.view_resource(item_id=item_id, view="map", lane="depth")
            assert payload["map"]["fills"][0]["name"] == "values"
            # These types retain the general petekTools value_layer fallback;
            # the renderer may itself compact an affine shell afterwards.
            assert not hasattr(surface, "_view_regular_grid")


@pytest.mark.parametrize("ncol,nrow", [(1, 1), (1, 4), (4, 1)])
def test_degenerate_regular_surface_legacy_views_materialize(ncol, nrow):
    surface = petekio.Surface.constant(
        petekio.GridGeometry(
            100.0,
            200.0,
            10.0,
            20.0,
            ncol,
            nrow,
            rotation_deg=15.0,
            yflip=True,
        ),
        10.0,
    )
    provider = ProjectViewProvider(_FakeProject({"Degenerate": surface}))
    leaf = next(_walk(_provider_tree(provider)))
    assert set(leaf["views"]) == {"map", "scene3d"}
    assert "transport" not in leaf["views"]["map"]
    assert leaf["visible"] == {"map": True, "scene3d": True}

    session = viewer.view(provider, serve=False)
    workspace_leaf = next(_walk(session.manifest()["workspace"]["tree"]))
    assert set(workspace_leaf["resources"]) == {"map", "scene3d"}
    mapped = session.resource("surface:Degenerate", "map")
    fill = mapped["payload"]["map"]["fills"][0]
    assert fill["regular_grid"]["dimensions"] == [ncol, nrow]
    scene = session.resource("surface:Degenerate", "scene3d", detail="preview")
    regular = scene["payload"]["scene3d"]["meshes"][0]["regular_surface"]
    assert regular["dimensions"] == [ncol, nrow]
    assert regular["triangle_count"] == 0


def _block_values(payload, marker, code):
    block = payload["map"]["blocks"][marker["__block__"]]
    raw = base64.b64decode(block["data"])
    return struct.unpack(f"<{len(raw) // struct.calcsize(code)}{code}", raw)


def _direct_block_values(block, code):
    raw = base64.b64decode(block["data"])
    return struct.unpack(f"<{len(raw) // struct.calcsize(code)}{code}", raw)


def _shared_block_values(resource, marker, code):
    return _direct_block_values(resource["blocks"][marker["__block__"]], code)


def test_regular_surface_map_is_native_affine_row_major_and_nan_exact():
    surface = petekio.Surface.load_irap_classic("tests/fixtures/simple.irap")
    surface.thickness = petekio.Surface.constant(surface.geometry, 7.0)
    provider = ProjectViewProvider(_FakeProject({"Rotated/Top": surface}))
    item_id = "surface:Rotated/Top"

    depth = provider.view_resource(item_id=item_id, view="map", detail="full")
    grid = depth["payload"]["map"]["surface_grid"]
    assert (grid["frame"]["ncol"], grid["frame"]["nrow"]) == (3, 4)
    assert grid["frame"]["origin_x"] == pytest.approx(1000.0)
    assert grid["frame"]["origin_y"] == pytest.approx(2000.0)
    assert grid["frame"]["spacing_x"] == pytest.approx(50.0)
    assert grid["frame"]["spacing_y"] == pytest.approx(25.0)
    assert grid["frame"]["rotation_deg"] == pytest.approx(30.0)
    assert grid["frame"]["crs"] is None and grid["frame"]["units"] == "m"
    assert grid["positive"] == "up"
    values = _shared_block_values(depth, grid["attributes"][0]["values"], "f")
    assert values[:5] == pytest.approx((1000.0, 1010.0, 1020.0, 1005.0, 1015.0))
    assert values[5] != values[5]
    assert set(_shared_block_values(depth, grid["mask"], "B")) == {1}
    assert set(_shared_block_values(depth, grid["attributes"][1]["values"], "f")) == {7.0}


def test_regular_surface_scene_has_progressive_compact_detail():
    geom = petekio.GridGeometry(
        1000.0, 2000.0, 25.0, 30.0, 750, 750, rotation_deg=23.0, yflip=True
    )
    surface = petekio.Surface.constant(geom, -2500.0)
    provider = ProjectViewProvider(_FakeProject({"Large": surface}))
    leaf = next(_walk(_provider_tree(provider)))
    assert leaf["views"]["map"]["tiers"] == [
        {"id": "preview", "label": "Preview"},
        {"id": "full", "label": "Full detail"},
    ]

    started = time.perf_counter()
    preview = provider.view_resource(
        item_id="surface:Large", view="map", detail="preview"
    )
    preview_seconds = time.perf_counter() - started
    preview_grid = preview["payload"]["map"]["surface_grid"]
    assert preview["detail"] == "preview"
    assert preview_grid["frame"]["ncol"] < 750
    assert preview_seconds < 0.25

    started = time.perf_counter()
    full = provider.view_resource(
        item_id="surface:Large", view="map", detail="full"
    )
    full_seconds = time.perf_counter() - started
    full_grid = full["payload"]["map"]["surface_grid"]
    assert [full_grid["frame"]["ncol"], full_grid["frame"]["nrow"]] == [750, 750]
    assert preview_grid["attributes"][0]["range"] == full_grid["attributes"][0]["range"]
    # The coarse preview spans the exact full footprint even though 749 is not
    # divisible by its stride; swapping detail must not change camera framing.
    assert preview_grid["frame"]["spacing_x"] * (preview_grid["frame"]["ncol"] - 1) == pytest.approx(
        full_grid["frame"]["spacing_x"] * 749
    )
    assert preview_grid["frame"]["spacing_y"] * (preview_grid["frame"]["nrow"] - 1) == pytest.approx(
        full_grid["frame"]["spacing_y"] * 749
    )
    assert full_seconds < 0.5

    started = time.perf_counter()
    map_payload = provider.view_resource(item_id="surface:Large", view="map")
    map_seconds = time.perf_counter() - started
    assert map_payload["payload"]["map"]["surface_grid"]["frame"]["yflip"] is True
    assert map_seconds < 0.5


def test_sparse_shared_preview_retains_a_finite_sample_and_stable_range(tmp_path):
    ncol = nrow = 500
    values = ["9999900.0"] * (ncol * nrow)
    values[ncol + 1] = "7.0"
    path = tmp_path / "sparse.irap"
    path.write_text(
        "\n".join(
            [
                f"-996 {nrow} 1 1",
                f"0 {ncol - 1} 0 {nrow - 1}",
                f"{ncol} 0 0 0",
                "0 0 0 0 0 0 0",
                " ".join(values),
            ]
        )
        + "\n"
    )
    surface = petekio.Surface.load_irap_classic(str(path))
    session = viewer.view(
        ProjectViewProvider(_FakeProject({"Sparse": surface})), serve=False
    )

    preview = session.resource("surface:Sparse", "map", detail="preview")
    full = session.resource("surface:Sparse", "map", detail="full")
    preview_grid = preview["payload"]["map"]["surface_grid"]
    full_grid = full["payload"]["map"]["surface_grid"]
    assert [preview_grid["frame"]["ncol"], preview_grid["frame"]["nrow"]] == [
        ncol,
        nrow,
    ]
    assert preview_grid["attributes"][0]["range"] == [7.0, 7.0]
    assert preview_grid["attributes"][0]["range"] == full_grid["attributes"][0]["range"]
    preview_values = _shared_block_values(
        preview, preview_grid["attributes"][0]["values"], "f"
    )
    assert [value for value in preview_values if value == value] == [7.0]


def test_native_regular_resources_never_call_full_mesh_duck():
    surface = _NativeSurfaceSpy("Native")
    provider = ProjectViewProvider(_FakeProject({"Native": surface}))

    provider.view_resource(item_id="surface:Native", view="map", detail="preview")
    assert surface.native_calls == [((None,), 1)]


def test_shared_surface_fetch_is_selector_free_linear_and_static_once(tmp_path):
    surface = _SharedMultiSurfaceSpy()
    provider = ProjectViewProvider(_FakeProject({"Shared": surface}))
    session = viewer.view(provider, serve=False)
    spec = next(_walk(session.manifest()["workspace"]["tree"]))["resources"]["map"]
    assert spec["transport"] == "shared" and spec["modes"] == ["2d", "3d"]
    assert [descriptor["id"] for descriptor in spec["attributes"]] == [
        "depth",
        "porosity",
        "facies",
    ]

    full = session.resource("surface:Shared", "map", detail="full")
    assert session.resource("surface:Shared", "map", detail="full") == full
    assert surface.shared_calls == [((None, "porosity", "facies"), 1)]
    grid = full["payload"]["map"]["surface_grid"]
    assert len(grid["attributes"]) == 3
    assert len(full["blocks"]) == 4  # one shell mask + one value block per lane
    assert grid["attributes"][1]["range"] == pytest.approx([0.1, 0.4])
    assert grid["attributes"][2]["range"] is None
    assert grid["attributes"][2]["kind"] == "categorical"
    with pytest.raises(ValueError, match="do not accept selectors"):
        session.resource(
            "surface:Shared",
            "map",
            detail="full",
            attribute="porosity",
            color_by="facies",
        )
    assert len(surface.shared_calls) == 1

    promoted_surface = _SharedMultiSurfaceSpy()
    promoted_provider = ProjectViewProvider(
        _FakeProject({"Shared": promoted_surface}), property="facies"
    )
    promoted_spec = next(_walk(promoted_provider.view_catalog()["tree"]))["views"][
        "map"
    ]
    assert promoted_spec["active_attribute"] == "facies"
    assert promoted_spec["active_color_by"] == "facies"
    promoted_provider.view_resource(
        item_id="surface:Shared", view="map", detail="full"
    )
    assert promoted_surface.shared_calls == [((None, "porosity", "facies"), 1)]

    static_surface = _SharedMultiSurfaceSpy()
    viewer.view(
        ProjectViewProvider(_FakeProject({"Shared": static_surface})), serve=False
    ).save(tmp_path / "shared-selected.html", include="selected")
    assert static_surface.shared_calls == [((None, "porosity", "facies"), 1)]


def test_promoted_categorical_primary_is_shared_geometry():
    geometry = petekio.GridGeometry(0.0, 0.0, 1.0, 1.0, 2, 2)
    source = petekio.Surface.constant(geometry, 100.0)
    metadata = {
        "id": "facies",
        "label": "Facies",
        "kind": "categorical",
        "units": None,
        "codes": {"1": {"label": "Sand", "color": "#EDA100"}},
    }
    source.set_attr(
        "facies", petekio.Surface.constant(geometry, 1.0), metadata=metadata
    )
    promoted = source.attr["facies"]
    provider = ProjectViewProvider(_FakeProject({"Facies": promoted}))
    spec = next(_walk(provider.view_catalog()["tree"]))["views"]["map"]
    assert spec["attributes"] == [metadata]
    assert spec["active_attribute"] == spec["active_color_by"] == "facies"
    resource = viewer.view(provider, serve=False).resource(
        "surface:Facies", "map", detail="full"
    )
    attribute = resource["payload"]["map"]["surface_grid"]["attributes"][0]
    assert attribute["kind"] == "categorical" and attribute["range"] is None


def test_shared_categorical_transport_requires_exact_f32_codes():
    geometry = petekio.GridGeometry(0.0, 0.0, 1.0, 1.0, 2, 2)

    def promoted(code):
        source = petekio.Surface.constant(geometry, 0.0)
        metadata = {
            "id": "facies",
            "label": "Facies",
            "kind": "categorical",
            "units": None,
            "codes": {str(code): {"label": "Boundary", "color": None}},
        }
        source.set_attr(
            "facies", petekio.Surface.constant(geometry, float(code)), metadata=metadata
        )
        return source.attr["facies"]

    exact_code = 2**24
    exact = viewer.view(
        ProjectViewProvider(_FakeProject({"Facies": promoted(exact_code)})),
        serve=False,
    ).resource("surface:Facies", "map", detail="full")
    grid = exact["payload"]["map"]["surface_grid"]
    attribute = grid["attributes"][0]
    assert attribute["codes"] == {
        str(exact_code): {"label": "Boundary", "color": None}
    }
    assert set(_shared_block_values(exact, attribute["values"], "f")) == {
        float(exact_code)
    }

    inexact_code = 2**24 + 1
    rejected = viewer.view(
        ProjectViewProvider(_FakeProject({"Facies": promoted(inexact_code)})),
        serve=False,
    )
    with pytest.raises(ValueError, match="not exactly representable as f32"):
        rejected.resource("surface:Facies", "map", detail="full")


def test_shared_continuous_transport_range_matches_f32_and_overflow_is_loud():
    geometry = petekio.GridGeometry(0.0, 0.0, 1.0, 1.0, 2, 2)
    f32_max = float.fromhex("0x1.fffffep+127")
    for index, source_value in enumerate((0.1, f32_max)):
        transported = struct.unpack("<f", struct.pack("<f", source_value))[0]
        resource = viewer.view(
            ProjectViewProvider(
                _FakeProject(
                    {f"Continuous {index}": petekio.Surface.constant(geometry, source_value)}
                )
            ),
            serve=False,
        ).resource(f"surface:Continuous%20{index}", "map", detail="full")
        attribute = resource["payload"]["map"]["surface_grid"]["attributes"][0]
        assert attribute["range"] == [transported, transported]
        assert set(_shared_block_values(resource, attribute["values"], "f")) == {
            transported
        }

    overflow = viewer.view(
        ProjectViewProvider(
            _FakeProject(
                {"Overflow": petekio.Surface.constant(geometry, float.fromhex("0x1p+128"))}
            )
        ),
        serve=False,
    )
    with pytest.raises(ValueError, match="outside the f32 transport range"):
        overflow.resource("surface:Overflow", "map", detail="full")


def test_shared_surface_detail_missing_and_malformed_data_are_local():
    detailed = _SharedMultiSurfaceSpy()
    session = viewer.view(
        ProjectViewProvider(_FakeProject({"Shared": detailed})), serve=False
    )
    session.resource("surface:Shared", "map", detail="preview")
    session.resource("surface:Shared", "map", detail="full")
    session.resource("surface:Shared", "map", detail="preview")
    assert len(detailed.shared_calls) == 2

    missing = ProjectViewProvider(
        _FakeProject({"Shared": _SharedMultiSurfaceSpy(missing_lane=True)})
    )
    with pytest.raises(ValueError, match="omitted a declared attribute"):
        missing.view_resource(item_id="surface:Shared", view="map", detail="full")

    malformed = viewer.view(
        ProjectViewProvider(
            _FakeProject(
                {"Shared": _SharedMultiSurfaceSpy(fractional_category=True)}
            )
        ),
        serve=False,
    )
    with pytest.raises(ValueError, match="values must be integral"):
        malformed.resource("surface:Shared", "map", detail="full")

    all_nan = viewer.view(
        ProjectViewProvider(
            _FakeProject({"Shared": _SharedMultiSurfaceSpy(all_nan=True)})
        ),
        serve=False,
    ).resource("surface:Shared", "map", detail="full")
    assert all_nan["payload"]["map"]["surface_grid"]["attributes"][1]["range"] is None


def test_scene_attribute_nulls_do_not_cut_finite_depth_geometry():
    attribute = petekio.Surface.load_irap_classic("tests/fixtures/simple.irap")
    primary = petekio.Surface.constant(attribute.geometry, -2500.0)
    primary.thickness = attribute
    provider = ProjectViewProvider(_FakeProject({"Top": primary}))

    payload = provider.view_resource(item_id="surface:Top", view="map", detail="full")
    regular = payload["payload"]["map"]["surface_grid"]
    mask = _shared_block_values(payload, regular["mask"], "B")
    elevations = _shared_block_values(payload, regular["attributes"][0]["values"], "f")
    values = _shared_block_values(payload, regular["attributes"][1]["values"], "f")
    assert mask[5] == 1 and values[5] != values[5]
    assert elevations[5] == -2500.0


def test_surface_map_overlay_emits_all_md_ordered_hits_and_is_cached():
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
    assert overlay["status"] == "ambiguous"
    assert overlay["intersection"] == {
        "md": 80.5,
        "xyz": [1080.5, 2000.0, -80.5],
    }
    assert overlay["intersections"] == [
        {"md": 40.25, "xyz": [1040.25, 2000.0, -40.25]},
        {"md": 80.5, "xyz": [1080.5, 2000.0, -80.5]},
    ]
    assert overlay["trajectory"][-1] == [1080.5, 2000.0, -80.5]
    assert "greatest-MD hit" in overlay["message"]

    provider.view_resource(item_id="surface:Top", view="map", lane="thickness")
    assert well._items[""].intersection_calls == 1
    validated = viewer.view(provider, serve=False).resource(
        "surface:Top", "map", attribute="depth", color_by="depth"
    )
    assert validated["payload"]["map"]["well_overlays"][0]["status"] == "ambiguous"


@pytest.mark.parametrize(
    "hits,status,count",
    [
        ([], "no_hit", 0),
        ([_Hit(20.0, (1020.0, 2000.0, -20.0))], "hit", 1),
        ([RuntimeError("boom")], "error", 0),
    ],
)
def test_surface_map_overlay_status_matches_intersection_cardinality(
    hits, status, count
):
    well = _OverlayWell(hits)
    provider = ProjectViewProvider(
        _FakeProject({"Top": _SurfaceSpy("Top")}, wells={"A": well}), lod=False
    )
    payload = provider.view_resource(item_id="surface:Top", view="map", lane="depth")
    overlay = payload["map"]["well_overlays"][0]
    assert overlay["status"] == status
    assert len(overlay["intersections"]) == count
    assert (overlay["intersection"] is not None) is (count == 1)
    validated = viewer.view(provider, serve=False).resource(
        "surface:Top", "map", attribute="depth", color_by="depth"
    )
    assert validated["payload"]["map"]["well_overlays"][0]["status"] == status


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
    leaves = list(_walk(_provider_tree(provider)))
    assert [leaf["id"] for leaf in leaves] == ["surface:A/Top", "surface:A/Base"]
    assert [leaf["visible"]["map"] for leaf in leaves] == [False, True]
    assert all(
        leaf["views"]["map"]["active_attribute"] == "thickness"
        and leaf["views"]["map"]["active_color_by"] == "thickness"
        for leaf in leaves
    )

    with pytest.raises(ValueError, match="ambiguous.*canonical full path"):
        ProjectViewProvider(
            _FakeProject(surfaces), selection={"surfaces": ["Top"]}
        )


def test_unknown_assets_are_disabled_and_preserved_in_catalog():
    project = _FakeProject(
        {"Top": _SurfaceSpy("Top")}, assets=["@asset/future/example"]
    )
    provider = ProjectViewProvider(project)
    unknown = next(
        leaf for leaf in _walk(_provider_tree(provider)) if leaf["role"] == "asset"
    )

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
    assert [leaf["id"] for leaf in _walk(_provider_tree(provider))] == [
        "surface:B/Base"
    ]
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


def test_static_visible_embeds_active_selector_and_selected_never_cartesian(tmp_path):
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
    with pytest.raises(ValueError, match="cannot enumerate.*non-shared"):
        viewer.view(selected_provider, serve=False).save(
            tmp_path / "selected.html", include="selected"
        )
    assert selected_spy.calls == []


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
    item_id = next(_walk(project.view_catalog()["tree"]))["id"]

    resource = project.view_resource(
        item_id=item_id, view="map", detail="preview"
    )
    assert resource["detail"] == "preview"
    assert "surface_grid" in resource["payload"]["map"]


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
