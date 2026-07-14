"""Lazy project-workspace producer for :class:`petekio.Project`.

petekIO owns project traversal and domain adaptation; petekTools owns the
generic workspace renderer.  Catalog construction is metadata-only.  The
optional renderer is imported only when a resource is materialized, served, or
saved.
"""

from __future__ import annotations

import copy
from collections.abc import Iterable, Mapping, Sequence
from pathlib import Path
from typing import TYPE_CHECKING, Any

from ._project_view_catalog import (
    ROLE_ALIASES,
    ROOTS,
    VIEWS,
    Entry,
    Folder,
    bore_id,
    label as catalog_label,
    typed_id,
)
from ._project_view_curves import select_auto_curves, template_curve_names
from ._project_view_surface import SurfaceViewResources
from ._specs import ViewSettings, ViewSpec

if TYPE_CHECKING:
    from ._project import Project


def _viewer():
    try:
        from petektools import viewer
    except (ImportError, AttributeError) as exc:
        raise ImportError(
            "project.view() renders through the optional petekTools workspace. "
            "Install it with `pip install 'petekio[toolkit]'`. Catalog inspection "
            "remains available with ViewSettings(serve=False)."
        ) from exc
    if not callable(getattr(viewer, "view", None)):
        raise ImportError(
            "the installed petekTools predates workspace views; install a build "
            "providing petektools.viewer.view and workspace schema v2"
        )
    return viewer


class _NamedPoints:
    kind = "point_set"

    def __init__(self, name: str, rows: Iterable[Sequence[float]]) -> None:
        self.name = name
        self._rows = [tuple(float(v) for v in row[:3]) for row in rows]

    def xyz(self) -> list[tuple[float, float, float]]:
        return list(self._rows)


class ProjectViewProvider:
    """Metadata snapshot plus lazy resource materializers for one project view."""

    def __init__(
        self,
        project: "Project",
        *,
        selection: Any = None,
        visible: Any = None,
        property: str | Mapping[str, str] | None = None,
        logs: ViewSpec | None = None,
        template: Any = None,
        lod: bool | tuple[int, ...] = True,
    ) -> None:
        if logs is not None and not isinstance(logs, ViewSpec):
            raise TypeError("project.view logs= must be a ViewSpec or None")
        self.project = project
        self.selection = selection
        self.explicit_visible = visible
        self.property = property
        self.logs = logs
        self.template = self._template(template)
        self.lod = lod
        self._entries: dict[str, Entry] = {}
        self._catalog: list[dict[str, Any]] = []
        self._diagnostics: list[dict[str, Any]] = []
        self._surface_resources = SurfaceViewResources(project, _viewer)
        self._auto_curves: dict[str, tuple[str, ...]] = {}
        self._snapshot(strict=True)

    @property
    def diagnostics(self) -> tuple[dict[str, Any], ...]:
        return tuple(copy.deepcopy(self._diagnostics))

    def _template(self, value: Any) -> Any:
        if isinstance(value, str):
            if value not in self.project.templates.all_names():
                raise KeyError(f"project.view template {value!r} was not found")
        return value

    def refresh(self) -> None:
        self._snapshot(strict=False)

    def view_catalog(self) -> dict[str, Any]:
        catalog: dict[str, Any] = {
            "schema_version": 2,
            "tree": copy.deepcopy(self._catalog),
        }
        title = getattr(self.project, "display_name", None)
        if title is not None:
            catalog["project"] = {
                "title": title,
                "crs": getattr(self.project, "crs", None),
                "unit": getattr(self.project, "unit", None),
            }
        return catalog

    def catalog_tree(self) -> list[dict[str, Any]]:
        return copy.deepcopy(self._catalog)

    def _surface_descriptors(
        self, surface: Any
    ) -> tuple[list[dict[str, Any]], dict[str, str | None]]:
        primary = getattr(surface, "primary_metadata", None)
        if primary is None:
            primary = {
                "id": "depth",
                "label": "Depth",
                "kind": "continuous",
                "units": getattr(self.project, "unit", None),
                "codes": None,
            }
        else:
            primary = dict(primary)
        descriptors = [primary]
        lane_attrs: dict[str, str | None] = {str(primary["id"]): None}
        metadata_getter = getattr(surface, "attr_metadata", None)
        for attr in surface.attr_names():
            metadata = (
                dict(metadata_getter(attr))
                if callable(metadata_getter)
                else {
                    "id": attr,
                    "label": catalog_label(attr),
                    "kind": "continuous",
                    "units": None,
                    "codes": None,
                }
            )
            selector = str(metadata["id"])
            if selector in lane_attrs:
                # A named legacy lane can collide with the reserved ordinary
                # primary selector. Keep it addressable without reordering it.
                selector = f"attribute:{selector}"
                metadata["id"] = selector
            descriptors.append(metadata)
            lane_attrs[selector] = attr
        return descriptors, lane_attrs

    def _snapshot(self, *, strict: bool) -> None:
        diagnostics: list[dict[str, Any]] = []
        entries = self._discover(diagnostics)
        selected = self._select(entries, self.selection, strict=strict, diagnostics=diagnostics)
        self._set_properties(selected, strict=strict, diagnostics=diagnostics)
        self._set_visibility(selected, self.explicit_visible, strict=strict, diagnostics=diagnostics)
        self._entries = {entry.id: entry for entry in selected}
        roots: list[dict[str, Any]] = []
        for role, root_label in ROOTS:
            members = [entry for entry in selected if entry.role == role]
            if not members:
                continue
            root = Folder("group:" + role, root_label)
            for entry in members:
                root.insert(entry)
            roots.append(root.to_dict())
        self._catalog = roots
        self._diagnostics = diagnostics
        self._surface_resources.reset(self._entries, self._diagnostics)

    def _discover(self, diagnostics: list[dict[str, Any]]) -> list[Entry]:
        entries: list[Entry] = []
        first_surface = True
        for name in self.project.surfaces.all_names():
            try:
                surface = self.project.surface(name)
                if surface is None:
                    raise KeyError(name)
                descriptors, lane_attrs = self._surface_descriptors(surface)
                active = next(
                    selector for selector, attr in lane_attrs.items() if attr is None
                )
                shared = (
                    callable(getattr(surface, "_view_shared_regular_grid", None))
                    and int(getattr(surface, "ncol", 0)) >= 2
                    and int(getattr(surface, "nrow", 0)) >= 2
                )
                if shared:
                    views = {
                        "map": {
                            "attributes": copy.deepcopy(descriptors),
                            "active_attribute": active,
                            "active_color_by": active,
                            "transport": "shared",
                            "modes": ["2d", "3d"],
                            "tiers": [
                                {"id": "preview", "label": "Preview"},
                                {"id": "full", "label": "Full detail"},
                            ],
                            "active_detail": "preview",
                        }
                    }
                    visible = {"map": first_surface}
                else:
                    views = {
                        view: {
                            "attributes": copy.deepcopy(descriptors),
                            "active_attribute": active,
                            "active_color_by": active,
                        }
                        for view in ("map", "scene3d")
                    }
                    if callable(getattr(surface, "_view_regular_grid", None)):
                        views["scene3d"].update(
                            tiers=[
                                {"id": "preview", "label": "Preview"},
                                {"id": "full", "label": "Full detail"},
                            ],
                            active_detail="preview",
                        )
                    visible = {"map": first_surface, "scene3d": first_surface}
                entries.append(
                    Entry(
                        typed_id("surface", name.split("/")),
                        name.rsplit("/", 1)[-1],
                        "surface",
                        tuple(name.split("/")),
                        (name,),
                        views,
                        visible,
                        lane_attrs=lane_attrs,
                    )
                )
                first_surface = False
            except Exception as exc:
                entries.append(self._disabled("surface", name, exc))
                diagnostics.append(self._diag("catalog_error", "surface", name, exc))

        for role, names in (
            ("point", self.project.points.all_names()),
            ("polygon", self.project.polygons.all_names()),
        ):
            for name in names:
                entries.append(
                    Entry(
                        typed_id(role, name.split("/")),
                        name.rsplit("/", 1)[-1],
                        role,
                        tuple(name.split("/")),
                        (name,),
                        {"map": {}, "scene3d": {}},
                        {"map": False, "scene3d": False},
                    )
                )

        bore_rows: list[tuple[str, str, str, list[str]]] = []
        for well_id in self.project.wells.names():
            well = self.project.well(well_id)
            if well is None:
                diagnostics.append(self._diag("missing_well", "bore", well_id, KeyError(well_id)))
                continue
            try:
                bores = list(well.bores()) or [""]
            except Exception as exc:
                diagnostics.append(self._diag("catalog_error", "bore", well_id, exc))
                continue
            for bore in bores:
                label = "Main" if not bore else str(bore)
                try:
                    sidetrack = well.sidetrack(bore)
                    mnemonics = [] if sidetrack is None else list(sidetrack.mnemonics())
                except Exception as exc:
                    diagnostics.append(self._diag("catalog_error", "bore", well_id, exc))
                    mnemonics = []
                bore_rows.append((well_id, bore, label, mnemonics))

        metadata = {
            bore_id(well_id, bore): mnemonics
            for well_id, bore, _, mnemonics in bore_rows
        }
        self._auto_curves = select_auto_curves(metadata)
        for well_id, bore, label, mnemonics in bore_rows:
            views: dict[str, dict[str, Any]] = {"map": {}, "scene3d": {}}
            visible = {"map": True, "scene3d": True}
            if self.logs is not None or mnemonics:
                views["wells"] = {}
                # Correlation data is deliberately opt-in even when its
                # metadata is discovered automatically: an initially
                # visible bore must never gather every log sample.
                visible["wells"] = False
            entries.append(
                Entry(
                    bore_id(well_id, bore),
                    label,
                    "bore",
                    (well_id, label),
                    (well_id, bore),
                    views,
                    visible,
                )
            )

        for name in self.project.well_tops.all_names():
            entries.append(
                Entry(
                    typed_id("well_top", name.split("/")),
                    name.rsplit("/", 1)[-1],
                    "well_top",
                    tuple(name.split("/")),
                    (name,),
                    {"map": {}, "scene3d": {}},
                    {"map": False, "scene3d": False},
                )
            )

        for name in self.project.tops.all_names():
            reason = "Imported top tables are source metadata; use project.well_tops for persisted picks."
            entries.append(self._disabled("source_top", name, reason))
        template_physical = {"@asset/templates/" + name for name in self.project.templates.all_names()}
        for name in self.project.templates.all_names():
            reason = "Correlation templates are presentation assets; select one with template=."
            entries.append(self._disabled("template", name, reason))
        for name in self.project.geodata.asset_names():
            if name in template_physical:
                continue
            reason = "No installed viewer provider declares a compatible view for this opaque asset."
            entry = self._disabled("asset", name, reason)
            entries.append(entry)
            diagnostics.append(
                {
                    "code": "unsupported_asset",
                    "severity": "warning",
                    "item_id": entry.id,
                    "message": reason,
                }
            )
        return entries

    def _disabled(self, role: str, name: str, reason: Any) -> Entry:
        message = str(reason)
        path = tuple(name.split("/"))
        diagnostic = {"code": "disabled", "severity": "info", "message": message}
        return Entry(
            typed_id(role, path),
            path[-1],
            role,
            path,
            (name,),
            {},
            {},
            disabled=True,
            reason=message,
            diagnostic=diagnostic,
        )

    @staticmethod
    def _diag(code: str, role: str, name: str, exc: Exception) -> dict[str, Any]:
        return {
            "code": code,
            "severity": "error",
            "role": role,
            "path": name,
            "error": type(exc).__name__,
            "message": str(exc),
        }

    def _select(
        self,
        entries: list[Entry],
        value: Any,
        *,
        strict: bool,
        diagnostics: list[dict[str, Any]],
    ) -> list[Entry]:
        if value is None:
            return entries
        selected: set[str] = set()
        if isinstance(value, Mapping):
            for raw_role, selectors in value.items():
                roles = ROLE_ALIASES.get(str(raw_role).lower())
                if roles is None:
                    raise ValueError(f"unknown project view role {raw_role!r}")
                scoped = [entry for entry in entries if entry.role in roles]
                if selectors is True:
                    selected.update(entry.id for entry in scoped)
                elif selectors not in (False, None):
                    selected.update(
                        self._resolve(scoped, selectors, strict=strict, diagnostics=diagnostics)
                    )
        else:
            selected.update(self._resolve(entries, value, strict=strict, diagnostics=diagnostics))
        return [entry for entry in entries if entry.id in selected]

    def _resolve(
        self,
        entries: list[Entry],
        selectors: Any,
        *,
        strict: bool,
        diagnostics: list[dict[str, Any]],
    ) -> set[str]:
        if selectors is True:
            return {entry.id for entry in entries}
        if selectors in (False, None):
            return set()
        values = [selectors] if isinstance(selectors, (str, bytes)) else list(selectors)
        out: set[str] = set()
        for raw in values:
            key = str(raw)
            exact = [entry for entry in entries if entry.id == key]
            if exact:
                out.add(exact[0].id)
                continue
            folder = key.endswith("/")
            plain = key[:-1] if folder else key
            matches = [
                entry
                for entry in entries
                if ("/".join(entry.path).startswith(plain + "/") if folder else "/".join(entry.path) == plain)
            ]
            if not matches and "/" not in plain and ":" not in plain:
                matches = [entry for entry in entries if entry.label == plain]
            if len(matches) == 1 or (folder and matches):
                out.update(entry.id for entry in matches)
                continue
            if len(matches) > 1:
                raise ValueError(
                    f"ambiguous project view selector {key!r}; use a canonical full path or typed ID"
                )
            message = f"project view selector {key!r} no longer matches a catalog item"
            if strict:
                raise KeyError(message)
            diagnostics.append({"code": "selection_missing", "severity": "warning", "message": message})
        return out

    def _set_properties(
        self, entries: list[Entry], *, strict: bool, diagnostics: list[dict[str, Any]]
    ) -> None:
        surfaces = [entry for entry in entries if entry.role == "surface" and not entry.disabled]
        if self.property is None:
            return
        assignments: list[tuple[Entry, str]] = []
        if isinstance(self.property, str):
            assignments = [(entry, self.property) for entry in surfaces]
        elif isinstance(self.property, Mapping):
            for selector, lane in self.property.items():
                ids = self._resolve(surfaces, selector, strict=strict, diagnostics=diagnostics)
                assignments.extend((entry, str(lane)) for entry in surfaces if entry.id in ids)
        else:
            raise TypeError("project.view property= must be a lane name, mapping, or None")
        for entry, requested in assignments:
            if requested in {"depth", "values"}:
                lane = next(
                    selector
                    for selector, attr in entry.lane_attrs.items()
                    if attr is None
                )
            elif requested in entry.lane_attrs:
                lane = requested
            else:
                lane = next(
                    (
                        selector
                        for selector, attr in entry.lane_attrs.items()
                        if attr == requested
                    ),
                    requested,
                )
            if lane not in entry.lane_attrs:
                message = f"surface {entry.id!r} has no property lane {requested!r}"
                if strict:
                    raise KeyError(message)
                diagnostics.append({"code": "property_missing", "severity": "warning", "item_id": entry.id, "message": message})
                continue
            for options in entry.views.values():
                options["active_attribute"] = lane
                options["active_color_by"] = lane

    def _set_visibility(
        self,
        entries: list[Entry],
        value: Any,
        *,
        strict: bool,
        diagnostics: list[dict[str, Any]],
    ) -> None:
        if value is None:
            return
        if isinstance(value, Mapping) and set(map(str, value)).issubset(VIEWS):
            for entry in entries:
                for view in entry.visible:
                    entry.visible[view] = False
            for view, selectors in value.items():
                ids = self._resolve(entries, selectors, strict=strict, diagnostics=diagnostics)
                for entry in entries:
                    if entry.id in ids and view in entry.visible:
                        entry.visible[str(view)] = True
            return
        ids = self._resolve(entries, value, strict=strict, diagnostics=diagnostics)
        for entry in entries:
            for view in entry.visible:
                entry.visible[view] = entry.id in ids

    def view_resource(
        self,
        *,
        item_id: str,
        view: str,
        lane: str | None = None,
        detail: str | None = None,
        attribute: str | None = None,
        color_by: str | None = None,
    ) -> dict[str, Any]:
        entry = self._entries.get(item_id)
        if entry is None:
            raise KeyError(f"unknown project workspace item {item_id!r}; call refresh()")
        if entry.disabled:
            raise ValueError(entry.reason or f"workspace item {item_id!r} is disabled")
        if view not in entry.views:
            raise KeyError(f"workspace item {item_id!r} has no {view!r} resource")
        if detail is not None and not entry.views[view].get("tiers"):
            raise KeyError(f"workspace item {item_id!r} has no {view!r} detail tiers")
        if entry.role == "surface":
            if entry.views[view].get("transport") == "shared":
                if lane is not None or attribute is not None or color_by is not None:
                    raise ValueError("shared surface resources do not accept selectors")
                return self._shared_surface(entry, detail)
            if lane is not None and (attribute is not None or color_by is not None):
                raise ValueError("surface resource cannot mix lane with attribute/color_by")
            if (attribute is None) != (color_by is None):
                raise ValueError("surface resource requires both attribute and color_by")
            if attribute is not None:
                if attribute != color_by:
                    raise ValueError(
                        "independent surface attribute/color_by materialization is "
                        "reserved for the shared Phase 6 transport"
                    )
                lane = attribute
            return self._surface(entry, view, lane, detail)
        if entry.role in {"point", "polygon"}:
            return self._spatial(entry, view)
        if entry.role == "bore":
            return self._bore(entry, view)
        if entry.role == "well_top":
            return self._well_top(entry, view)
        raise ValueError(entry.reason or f"no materializer for {entry.role!r}")

    def _shared_surface(
        self, entry: Entry, detail: str | None
    ) -> dict[str, Any]:
        obj = self.project.surface(entry.source[0])
        if obj is None:
            raise KeyError(f"surface {entry.source[0]!r} was renamed or deleted; call refresh()")
        transport = getattr(obj, "_view_shared_regular_grid", None)
        if not callable(transport):
            raise ValueError(f"surface {entry.id!r} no longer supports shared transport")
        stride = self._surface_resources.preview_stride(obj) if detail == "preview" else 1
        grid = transport(attrs=list(entry.lane_attrs.values()), stride=stride)
        payload = self._surface_resources.shared_regular(
            entry,
            grid,
            copy.deepcopy(entry.views["map"]["attributes"]),
            detail,
        )
        self._surface_resources.attach_well_overlays(
            entry, obj, payload["payload"]
        )
        return payload

    def _surface(
        self, entry: Entry, view: str, lane: str | None, detail: str | None
    ) -> dict[str, Any]:
        obj = self.project.surface(entry.source[0])
        if obj is None:
            raise KeyError(f"surface {entry.source[0]!r} was renamed or deleted; call refresh()")
        lane = lane or entry.views[view]["active_attribute"]
        if lane not in entry.lane_attrs:
            raise KeyError(f"surface {entry.id!r} has no declared lane {lane!r}")
        attr = entry.lane_attrs[lane]
        regular = getattr(obj, "_view_regular_grid", None)
        if callable(regular):
            if view == "map":
                payload = self._surface_resources.regular_map(
                    entry, regular(attr=attr, stride=1)
                )
                self._surface_resources.attach_well_overlays(entry, obj, payload)
                return payload
            stride = (
                self._surface_resources.preview_stride(obj) if detail == "preview" else 1
            )
            return self._surface_resources.regular_scene(
                entry, regular(attr=attr, stride=stride), detail
            )
        fill: bool | str = True if attr is None else attr
        item = {"object": obj, "id": entry.id, "name": entry.label, "fill": fill}
        viewer = _viewer()
        if view == "map":
            payload = viewer.view2d_payload([item], title=entry.label, lod=self.lod)
            self._surface_resources.attach_well_overlays(entry, obj, payload)
            return payload
        return viewer.view3d_payload([item], title=entry.label)

    def _spatial(self, entry: Entry, view: str) -> dict[str, Any]:
        getter = self.project.point_set if entry.role == "point" else self.project.polygon_set
        obj = getter(entry.source[0])
        if obj is None:
            raise KeyError(f"{entry.role} {entry.source[0]!r} was renamed or deleted; call refresh()")
        item = {"object": obj, "id": entry.id, "name": entry.label}
        viewer = _viewer()
        return (
            viewer.view2d_payload([item], title=entry.label, lod=self.lod)
            if view == "map"
            else viewer.view3d_payload([item], title=entry.label)
        )

    def _bore(self, entry: Entry, view: str) -> dict[str, Any]:
        well_id, bore = entry.source
        well = self.project.well(well_id)
        sidetrack = None if well is None else well.sidetrack(bore)
        if sidetrack is None:
            raise KeyError(f"bore {well_id!r}/{bore!r} was renamed or deleted; call refresh()")
        if view == "wells":
            spec = self.logs
            if spec is None:
                template_curves = self._template_curves()
                curves = (
                    template_curves
                    if template_curves is not None
                    else self._auto_curves.get(entry.id, ())
                )
                spec = ViewSpec(curves=curves, tops=True)
            curves = spec.curves
            raw = sidetrack._view_raw(curves)
            raw["id"] = entry.id
            raw["display_name"] = f"{well_id} / {entry.label}"
            from ._viewer import LogSession, _materialize_template, build_well_log_bundle

            bundle = build_well_log_bundle([raw], spec=spec)
            if self.template is not None:
                template = (
                    self.project.templates[self.template]
                    if isinstance(self.template, str)
                    else self.template
                )
                bundle = _materialize_template(template).apply(bundle)
            return LogSession(bundle)._viewer_payload()
        rows = [
            point
            for _, point in self._surface_resources.trajectory_samples(
                entry.id, sidetrack
            )
        ]
        head = well.head
        wire = {
            "id": entry.id,
            "display_name": f"{well_id} / {entry.label}",
            "x": head[0],
            "y": head[1],
            "trajectory": rows,
        }
        viewer = _viewer()
        payload = (
            viewer.view2d_payload([], title=entry.label, wells=[wire], well_labels="auto")
            if view == "map"
            else viewer.view3d_payload([], title=entry.label, wells=[wire], well_labels="auto")
        )
        for rendered in payload.get("wells", []):
            rendered["item_id"] = entry.id
        for rendered in (payload.get("scene3d") or {}).get("wells", []):
            rendered["item_id"] = entry.id
        return payload

    def _template_curves(self) -> tuple[str, ...] | None:
        if self.template is None:
            return None
        template = (
            self.project.templates[self.template]
            if isinstance(self.template, str)
            else self.template
        )
        return template_curve_names(template)

    def _well_top(self, entry: Entry, view: str) -> dict[str, Any]:
        try:
            top_set = self.project.well_tops[entry.source[0]]
        except KeyError as exc:
            raise KeyError(f"well top {entry.source[0]!r} was renamed or deleted; call refresh()") from exc
        points = _NamedPoints(entry.label, (row["xyz"] for row in top_set.rows))
        item = {"object": points, "id": entry.id, "name": entry.label}
        viewer = _viewer()
        return (
            viewer.view2d_payload([item], title=entry.label, lod=self.lod)
            if view == "map"
            else viewer.view3d_payload([item], title=entry.label)
        )


class ProjectViewSession:
    """Inspectable lazy workspace returned by :meth:`Project.view`."""

    def __init__(self, provider: ProjectViewProvider, *, tab: str = "auto") -> None:
        self._provider = provider
        self._tab = tab
        self._workspace: Any = None

    @property
    def url(self) -> str | None:
        return None if self._workspace is None else self._workspace.url

    @property
    def diagnostics(self) -> tuple[dict[str, Any], ...]:
        values = list(self._provider.diagnostics)
        if self._workspace is not None:
            values.extend(self._workspace.diagnostics)
        return tuple(copy.deepcopy(values))

    def tree(self) -> list[dict[str, Any]]:
        return self._provider.catalog_tree()

    def _session(self):
        if self._workspace is None:
            self._workspace = _viewer().view(
                self._provider,
                title=getattr(self._provider.project, "display_name", None)
                or "Project workspace",
                tab=self._tab,
                serve=False,
            )
        return self._workspace

    def resource(
        self,
        item_id: str,
        view: str,
        lane: str | None = None,
        detail: str | None = None,
        *,
        attribute: str | None = None,
        color_by: str | None = None,
    ) -> dict[str, Any]:
        if lane is not None:
            if attribute is not None or color_by is not None:
                raise ValueError("surface resource cannot mix lane with attribute/color_by")
            attribute = color_by = lane
            lane = None
        return self._session().resource(
            item_id,
            view,
            lane,
            detail,
            attribute=attribute,
            color_by=color_by,
        )

    def manifest(self) -> dict[str, Any]:
        return self._session().manifest()

    def serve(self, **kwargs: Any) -> "ProjectViewSession":
        self._session().serve(**kwargs)
        return self

    def save(self, path: str | Path, *, include: str = "visible") -> "ProjectViewSession":
        self._session().save(path, include=include)
        return self

    def refresh(self) -> "ProjectViewSession":
        self._provider.refresh()
        if self._workspace is not None:
            self._workspace.refresh()
        return self


def project_view(
    project: "Project",
    selection: Any = None,
    *,
    visible: Any = None,
    property: str | Mapping[str, str] | None = None,
    logs: ViewSpec | None = None,
    template: Any = None,
    tab: str = "auto",
    lod: bool | tuple[int, ...] = True,
    settings: ViewSettings | None = None,
) -> ProjectViewSession:
    if settings is not None and not isinstance(settings, ViewSettings):
        raise TypeError("project.view settings= must be ViewSettings or None")
    if tab != "auto" and tab not in VIEWS:
        raise ValueError(f"unknown project view tab {tab!r}")
    provider = ProjectViewProvider(
        project,
        selection=selection,
        visible=visible,
        property=property,
        logs=logs,
        template=template,
        lod=lod,
    )
    session = ProjectViewSession(provider, tab=tab)
    delivery = settings or ViewSettings()
    if delivery.save is not None:
        return session.save(delivery.save)
    if delivery.serve:
        return session.serve()
    return session


__all__ = ["ProjectViewProvider", "ProjectViewSession", "project_view"]
