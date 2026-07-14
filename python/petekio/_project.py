"""Project-level raw-tree import and compact-project loading facade for petekIO.

`Project` is intentionally a Python facade over `GeoData`: it owns scan metadata
and user-facing inventory, while all loaded subsurface data remains in the Rust
`GeoData` substrate.
"""

from __future__ import annotations

import json
import math
import re
import shlex
from collections.abc import Iterable, Iterator, Mapping, MutableMapping
from dataclasses import dataclass, field
from math import isfinite
from pathlib import Path
from typing import Any

from ._petekio import FormatKind, GeoData, IngestSpec, detect


SKIP_UNSUPPORTED_FORMAT = "unsupported_format"
SKIP_AMBIGUOUS_GEOJSON = "ambiguous_geojson"
SKIP_IMPORT_ERROR = "import_error"

_TEMPLATE_ASSET_PREFIX = "@asset/templates/"
_ASSET_FRAME_VERSION = 1
_TEMPLATE_CODEC = "application/json"


@dataclass(frozen=True)
class _FileRecord:
    path: Path
    kind: FormatKind


@dataclass(frozen=True)
class ImportSettings:
    """Project import settings owned by petekIO.

    ``Project.import_data(..., settings=ImportSettings(...))`` is the
    notebook-facing form for raw source trees. ``Project.load(...)`` is reserved
    for compact ``.pproj`` files.
    """

    crs: str | None = None
    aliases: Mapping[str, Any] | None = None
    unit: str = "m"
    options: Mapping[str, Any] = field(default_factory=dict)

    def to_mapping(self) -> dict[str, Any]:
        out = dict(self.options)
        if self.unit:
            out["unit"] = self.unit
        return out


class _NamedCollection:
    """Folder-aware project names with optional lookup by name."""

    def __init__(self, names: Iterable[str], getter, *, prefix: str = ""):
        self._names = [_normalise_project_path(name) for name in names]
        self._getter = getter
        self._prefix = _normalise_project_path(prefix)

    def __len__(self) -> int:
        return len(self._visible_names())

    def __iter__(self) -> Iterator[str]:
        return iter(self._visible_names())

    def __contains__(self, name: object) -> bool:
        return isinstance(name, str) and (
            _resolve_collection_name(name, self._names, prefix=self._prefix) is not None
            or _resolve_collection_folder(name, self._names, prefix=self._prefix) is not None
        )

    def __getitem__(self, name: int | slice | str) -> Any:
        if isinstance(name, (int, slice)):
            return self._visible_names()[name]
        resolved = _resolve_collection_name(name, self._names, prefix=self._prefix)
        if resolved is not None:
            value = self._getter(resolved)
            if value is None:
                raise KeyError(resolved)
            return value
        folder = _resolve_collection_folder(name, self._names, prefix=self._prefix)
        if folder is not None:
            return self._descend(folder)
        raise KeyError(name)

    def __call__(self, name: str) -> Any:
        resolved = _resolve_collection_name(name, self._names, prefix=self._prefix)
        return None if resolved is None else self._getter(resolved)

    def get(self, name: str, default: Any = None) -> Any:
        resolved = _resolve_collection_name(name, self._names, prefix=self._prefix)
        if resolved is None:
            return default
        value = self._getter(resolved)
        return default if value is None else value

    def names(self) -> list[str]:
        return self._visible_names()

    def all_names(self) -> list[str]:
        return list(self._names)

    def folders(self) -> list[str]:
        return [name for name in self._visible_names() if name.endswith("/")]

    def objects(self) -> list[str]:
        return [name for name in self._visible_names() if not name.endswith("/")]

    def values(self) -> list[Any]:
        return [self._getter(name) for name in self._object_names_at_prefix()]

    def items(self) -> list[tuple[str, Any]]:
        return [(name.rsplit("/", 1)[-1], self._getter(name)) for name in self._object_names_at_prefix()]

    def _visible_names(self) -> list[str]:
        return _visible_collection_names(self._names, self._prefix)

    def _object_names_at_prefix(self) -> list[str]:
        return [
            name
            for name in self._names
            if _relative_to_project_prefix(name, self._prefix) is not None
            and "/" not in _relative_to_project_prefix(name, self._prefix)
        ]

    def _descend(self, prefix: str) -> "_NamedCollection":
        return _NamedCollection(self._names, self._getter, prefix=prefix)

    def __getattr__(self, name: str) -> Any:
        folder = _resolve_collection_folder(name, self._names, prefix=self._prefix)
        if folder is not None:
            return self._descend(folder)
        resolved = _resolve_collection_name(name, self._names, prefix=self._prefix)
        if resolved is not None:
            value = self._getter(resolved)
            if value is not None:
                return value
        raise AttributeError(name)

    def __eq__(self, other: object) -> bool:
        if isinstance(other, _NamedCollection):
            return self._visible_names() == other._visible_names()
        if isinstance(other, list):
            return self._visible_names() == other
        return False

    def __repr__(self) -> str:
        return repr(self._visible_names())

    __str__ = __repr__


class _TopsCollection:
    """Folder-aware top-set names with DataFrame lookup by set name."""

    def __init__(
        self,
        names: Iterable[str],
        rows_by_name: Mapping[str, list[dict[str, Any]]],
        *,
        prefix: str = "",
    ):
        self._names = [_normalise_project_path(name) for name in names]
        self._rows_by_name = {str(name): list(rows) for name, rows in rows_by_name.items()}
        self._prefix = _normalise_project_path(prefix)

    def __len__(self) -> int:
        return len(self._visible_names())

    def __iter__(self) -> Iterator[str]:
        return iter(self._visible_names())

    def __contains__(self, name: object) -> bool:
        return isinstance(name, str) and (
            _resolve_collection_name(name, self._names, prefix=self._prefix) is not None
            or _resolve_collection_folder(name, self._names, prefix=self._prefix) is not None
        )

    def __getitem__(self, name: int | slice | str) -> Any:
        if isinstance(name, (int, slice)):
            return self._visible_names()[name]
        resolved = _resolve_collection_name(name, self._names, prefix=self._prefix)
        if resolved is not None:
            return _dataframe(self._rows_by_name.get(resolved, []))
        folder = _resolve_collection_folder(name, self._names, prefix=self._prefix)
        if folder is not None:
            return self._descend(folder)
        raise KeyError(name)

    def get(self, name: str, default: Any = None) -> Any:
        try:
            return self[name]
        except KeyError:
            return default

    def names(self) -> list[str]:
        return self._visible_names()

    def all_names(self) -> list[str]:
        return list(self._names)

    def items(self) -> list[tuple[str, Any]]:
        return [
            (name.rsplit("/", 1)[-1], _dataframe(self._rows_by_name.get(name, [])))
            for name in self._object_names_at_prefix()
        ]

    def _visible_names(self) -> list[str]:
        return _visible_collection_names(self._names, self._prefix)

    def _object_names_at_prefix(self) -> list[str]:
        return [
            name
            for name in self._names
            if _relative_to_project_prefix(name, self._prefix) is not None
            and "/" not in _relative_to_project_prefix(name, self._prefix)
        ]

    def _descend(self, prefix: str) -> "_TopsCollection":
        return _TopsCollection(self._names, self._rows_by_name, prefix=prefix)

    def __getattr__(self, name: str) -> Any:
        folder = _resolve_collection_folder(name, self._names, prefix=self._prefix)
        if folder is not None:
            return self._descend(folder)
        resolved = _resolve_collection_name(name, self._names, prefix=self._prefix)
        if resolved is not None:
            return _dataframe(self._rows_by_name.get(resolved, []))
        raise AttributeError(name)

    def __eq__(self, other: object) -> bool:
        if isinstance(other, _TopsCollection):
            return self._visible_names() == other._visible_names()
        if isinstance(other, list):
            return self._visible_names() == other
        return False

    def __repr__(self) -> str:
        return repr(self._visible_names())

    __str__ = __repr__


@dataclass(frozen=True)
class WellTopSet:
    """One persisted formation horizon aggregated across project well bores."""

    name: str
    rows: tuple[dict[str, Any], ...]

    def __len__(self) -> int:
        return len(self.rows)

    def __iter__(self) -> Iterator[dict[str, Any]]:
        return iter(self.rows)

    def __getitem__(self, index: int | slice) -> Any:
        return self.rows[index]

    def summary(self) -> dict[str, int]:
        return {
            "picks": len(self.rows),
            "wells": len({row["well"] for row in self.rows}),
            "bores": len({(row["well"], row["bore"]) for row in self.rows}),
        }

    def to_dataframe(self) -> Any:
        return _dataframe(list(self.rows))

    def __repr__(self) -> str:
        return f"WellTopSet(name={self.name!r}, picks={len(self.rows)})"


class _WellTopsMapping(MutableMapping[str, WellTopSet]):
    """Folder-aware mutable view over the actual persisted per-bore tops."""

    def __init__(self, project: "Project", *, prefix: str = "") -> None:
        self._project = project
        self._prefix = _normalise_project_path(prefix)

    def _names(self) -> list[str]:
        return list(self._project.geodata.well_top_names())

    def __len__(self) -> int:
        return len(self._visible_names())

    def __iter__(self) -> Iterator[str]:
        return iter(self._visible_names())

    def __getitem__(self, name: str) -> WellTopSet | "_WellTopsMapping":
        names = self._names()
        resolved = _resolve_collection_name(name, names, prefix=self._prefix)
        if resolved is not None:
            rows = []
            for well, bore, md, xyz in self._project.geodata.well_top_set(resolved):
                rows.append({"well": well, "bore": bore, "md": md, "xyz": xyz})
            return WellTopSet(resolved, tuple(rows))
        folder = _resolve_collection_folder(name, names, prefix=self._prefix)
        if folder is not None:
            return _WellTopsMapping(self._project, prefix=folder)
        raise KeyError(name)

    def __setitem__(self, name: str, value: Any) -> None:
        logical = _normalise_project_path(name)
        full_name = "/".join(part for part in (self._prefix, logical) if part)
        if not full_name:
            raise ValueError("well-top name cannot be empty")
        apply_top = getattr(value, "_apply_top", None)
        if not callable(apply_top):
            raise TypeError(
                "well-top assignment requires project.wells.intersection(surface); "
                "use bore.add_top(...) for an explicit single-bore pick"
            )
        # The compiled result validates same-project identity, complete-view
        # scope, failure-free diagnostics, unique bore hits, and every XYZ/MD
        # before its no-fail replacement pass mutates any Top record.
        apply_top(self._project.geodata, full_name)

    def __delitem__(self, name: str) -> None:
        names = self._names()
        resolved = _resolve_collection_name(name, names, prefix=self._prefix)
        if resolved is None:
            raise KeyError(name)
        removed = self._project.geodata.delete_well_top(resolved)
        if not removed:
            raise KeyError(resolved)

    def names(self) -> list[str]:
        return self._visible_names()

    def all_names(self) -> list[str]:
        return self._names()

    def folders(self) -> list[str]:
        return [name for name in self._visible_names() if name.endswith("/")]

    def values(self) -> list[WellTopSet]:
        return [self[name] for name in self._object_names_at_prefix()]  # type: ignore[list-item]

    def items(self) -> list[tuple[str, WellTopSet]]:
        return [
            (name.rsplit("/", 1)[-1], self[name])  # type: ignore[list-item]
            for name in self._object_names_at_prefix()
        ]

    def _visible_names(self) -> list[str]:
        return _visible_collection_names(self._names(), self._prefix)

    def _object_names_at_prefix(self) -> list[str]:
        return [
            name
            for name in self._names()
            if _relative_to_project_prefix(name, self._prefix) is not None
            and "/" not in _relative_to_project_prefix(name, self._prefix)
        ]

    def __getattr__(self, name: str) -> Any:
        try:
            return self[name]
        except KeyError as exc:
            raise AttributeError(name) from exc

    def __repr__(self) -> str:
        return repr(self._visible_names())


class _ProjectWellView:
    """Project well wrapper that exposes notebook-friendly log discovery."""

    def __init__(self, well_id: str, well: Any) -> None:
        self.id = well_id
        self._well = well

    @property
    def logs(self) -> list[str]:
        return _well_log_names(self._well)

    @property
    def raw(self) -> Any:
        return self._well

    def __getattr__(self, name: str) -> Any:
        return getattr(self._well, name)

    def __repr__(self) -> str:
        return f"Well({self.id!r})"


class _ProjectWellsView:
    """List-like project wells view with project-wide log discovery."""

    def __init__(self, project: "Project") -> None:
        self._project = project
        self._view = project.geodata.wells
        self._names = list(project._inventory.get("wells", []))

    @property
    def logs(self) -> Any:
        """Project-wide list-like lazy log-expression namespace."""

        from ._logs import Logs

        return Logs(self._project)

    def __len__(self) -> int:
        return len(self._names)

    def __iter__(self) -> Iterator[str]:
        return iter(self._names)

    def __contains__(self, well_id: object) -> bool:
        return well_id in self._names

    def __getitem__(self, well_id: int | slice | str) -> Any:
        if isinstance(well_id, (int, slice)):
            return self._names[well_id]
        well = self._project.well(well_id)
        if well is None:
            raise KeyError(well_id)
        return _ProjectWellView(well_id, well)

    def names(self) -> list[str]:
        return list(self._names)

    def values(self) -> list[_ProjectWellView]:
        return [self[name] for name in self._names]

    def items(self) -> list[tuple[str, _ProjectWellView]]:
        return [(name, self[name]) for name in self._names]

    def get(self, well_id: str, default: Any = None) -> Any:
        try:
            return self[well_id]
        except KeyError:
            return default

    def view(
        self,
        *args: Any,
        template: Any = None,
        wells: Iterable[str] | str | None = None,
        **kwargs: Any,
    ) -> Any:
        """Render this project's wells, optionally narrowed by well id.

        ``template`` is presentation state and remains independent of
        ``ViewSpec`` (WHAT) and ``ViewSettings`` (HOW).
        """

        target = self._view
        if wells is not None:
            requested = [wells] if isinstance(wells, str) else list(wells)
            selected: set[str] = set()
            for name in requested:
                if not isinstance(name, str):
                    raise TypeError("view wells must be well-id strings")
                resolved = _resolve_collection_name(name, self._names)
                if resolved is None:
                    raise KeyError(name)
                selected.add(resolved)
            target = target.filter(lambda well: well.id in selected)
        if template is not None:
            kwargs["template"] = template
        return target.view(*args, **kwargs)

    def assign_log(
        self,
        name: str,
        expr: Any,
        *,
        basis: Any = None,
        interpolation: str = "linear",
        overwrite: bool = False,
    ) -> "AssignLogResult":
        """Assign a calculated log across wells/bores.

        Without ``basis=`` every log operand must already share the same MD
        sampling. With ``basis=logs.PHIE``, all non-basis operands are resampled
        to PHIE's MD sampling using ``interpolation`` unless they already declared
        their own ``.to_basis(...)``.
        """

        from ._logs import LogChannel, LogExpression, LogBasis, _normalise_interpolation

        if not isinstance(expr, (LogChannel, LogExpression, LogBasis)):
            raise TypeError("assign_log expr must be a logs.* channel or log arithmetic expression")
        if basis is not None and not isinstance(basis, (LogChannel, LogBasis)):
            raise TypeError("assign_log basis must be a logs.* channel or .to_basis(...) expression")
        default_interpolation = _normalise_interpolation(interpolation)
        output = _clean_log_name(name)
        if not output:
            raise ValueError("assigned log name cannot be empty")

        created: list[dict[str, Any]] = []
        skipped: list[dict[str, Any]] = []
        failed: list[dict[str, Any]] = []

        for well_id in self._names:
            well = self._project.well(well_id)
            if well is None:
                skipped.append({"well": well_id, "reason": "missing_well"})
                continue
            bores = _call_names(well, "bores") or [""]
            for bore in bores:
                sidetrack = well.sidetrack(bore)
                if sidetrack is None:
                    skipped.append({"well": well_id, "bore": bore, "reason": "missing_bore"})
                    continue
                try:
                    target_md = None
                    if basis is not None:
                        target_md = _eval_log_operand(
                            basis,
                            sidetrack,
                            target_md=None,
                            default_interpolation=default_interpolation,
                        ).md
                    series = _eval_log_operand(
                        expr,
                        sidetrack,
                        target_md=target_md,
                        default_interpolation=default_interpolation,
                    )
                    if sidetrack.log(output) is not None and not overwrite:
                        raise ValueError(f"log '{output}' already exists on this bore")
                    sidetrack.assign_log(output, series.md, series.values, overwrite=overwrite)
                    created.append({
                        "well": well_id,
                        "bore": bore,
                        "log": output,
                        "samples": len(series.md),
                    })
                except _MissingLog as exc:
                    skipped.append({
                        "well": well_id,
                        "bore": bore,
                        "reason": "missing_log",
                        "log": exc.mnemonic,
                    })
                except Exception as exc:
                    failed.append({
                        "well": well_id,
                        "bore": bore,
                        "reason": type(exc).__name__,
                        "message": str(exc),
                    })

        result = AssignLogResult(created=created, skipped=skipped, failed=failed)
        if failed:
            first = failed[0]
            raise ValueError(
                f"assign_log failed for {len(failed)} bore(s); first failure "
                f"{first.get('well')}/{first.get('bore')}: {first.get('message')}"
            )
        return result

    def __getattr__(self, name: str) -> Any:
        well_id = _well_attr_lookup(name, self._names)
        if well_id is not None:
            return self[well_id]
        return getattr(self._view, name)

    def __eq__(self, other: object) -> bool:
        if isinstance(other, _ProjectWellsView):
            return self._names == other._names
        if isinstance(other, list):
            return self._names == other
        return False

    def __repr__(self) -> str:
        return repr(self._names)

    __str__ = __repr__


@dataclass(frozen=True, slots=True)
class BoundTemplate:
    """Immutable project-owned snapshot of one persisted view template."""

    _project: "Project" = field(repr=False, compare=False)
    name: str
    kind: str
    schema_version: int
    _provider: str = field(repr=False)
    _bytes: bytes = field(repr=False)

    def to_dict(self) -> dict[str, Any]:
        """Return a detached plain-data copy of the stored snapshot."""

        value = json.loads(self._bytes.decode("utf-8"))
        if not isinstance(value, dict):  # guarded on insertion/open; defensive
            raise ValueError(f"template '{self.name}' payload is not a JSON object")
        return value

    def materialize(self) -> Any:
        """Decode through the optional provider that owns template semantics."""

        if not self._provider.startswith("petektools.viewer."):
            raise ValueError(
                f"template '{self.name}' declares unsupported provider {self._provider!r}"
            )
        try:
            from petektools import viewer
        except (ImportError, AttributeError) as exc:
            raise ImportError(_template_provider_guidance(self.kind)) from exc
        template_type = getattr(viewer, self._provider.rsplit(".", 1)[-1], None)
        if template_type is None or not callable(getattr(template_type, "from_dict", None)):
            raise ImportError(_template_provider_guidance(self.kind))
        return template_type.from_dict(self.to_dict())

    def __call__(
        self,
        *,
        wells: Iterable[str] | str | None = None,
        spec: Any = None,
        settings: Any = None,
        **kwargs: Any,
    ) -> Any:
        """Render all project wells, or the explicitly selected well ids."""

        if spec is not None:
            kwargs["spec"] = spec
        if settings is not None:
            kwargs["settings"] = settings
        return self._project.wells.view(template=self, wells=wells, **kwargs)


class _TemplatesCollection(_NamedCollection):
    """Folder-aware mutable library of immutable project template snapshots."""

    def __init__(self, project: "Project", *, prefix: str = "") -> None:
        self._project = project
        super().__init__(self._logical_names(), project._bound_template, prefix=prefix)

    def _logical_names(self) -> list[str]:
        return [
            name[len(_TEMPLATE_ASSET_PREFIX) :]
            for name in self._project.geodata.asset_names()
            if name.startswith(_TEMPLATE_ASSET_PREFIX)
        ]

    def _refresh(self) -> None:
        self._names = self._logical_names()

    def _descend(self, prefix: str) -> "_TemplatesCollection":
        return _TemplatesCollection(self._project, prefix=prefix)

    def add(self, template: Any, *, tags: Iterable[str] = ()) -> BoundTemplate:
        """Add a named snapshot; fail if its intrinsic name already exists."""

        name, _, _, _, envelope, payload = _template_snapshot(template)
        physical = _template_physical_name(name)
        self._project.geodata.add_asset(
            physical,
            "template",
            envelope,
            [str(tag) for tag in tags],
            _ASSET_FRAME_VERSION,
            payload,
        )
        self._refresh()
        return self._project._bound_template(name)

    def replace(
        self,
        template: Any,
        *,
        tags: Iterable[str] | None = None,
    ) -> BoundTemplate:
        """Replace a named snapshot; fail if its intrinsic name is absent."""

        name, _, _, _, envelope, payload = _template_snapshot(template)
        physical = _template_physical_name(name)
        existing = self._project.geodata.asset(physical)
        if existing is None:
            raise KeyError(name)
        kept_tags = existing["tags"] if tags is None else [str(tag) for tag in tags]
        self._project.geodata.replace_asset(
            physical,
            "template",
            envelope,
            kept_tags,
            _ASSET_FRAME_VERSION,
            payload,
        )
        self._refresh()
        return self._project._bound_template(name)

    def rename(self, old: str, new: str) -> BoundTemplate:
        """Rename the physical asset and its intrinsic template name."""

        self._refresh()
        resolved = _resolve_collection_name(old, self._names, prefix=self._prefix)
        if resolved is None:
            raise KeyError(old)
        new_name = _validate_template_name(new)
        new_physical = _template_physical_name(new_name)
        if self._project.geodata.asset(new_physical) is not None:
            raise ValueError(f"template '{new_name}' already exists")
        bound = self._project._bound_template(resolved)
        data = bound.to_dict()
        data["name"] = new_name
        _, _, _, _, envelope, payload = _template_snapshot(data)
        old_physical = _template_physical_name(resolved)
        existing = self._project.geodata.asset(old_physical)
        assert existing is not None
        self._project.geodata.rename_asset(old_physical, new_physical)
        self._project.geodata.replace_asset(
            new_physical,
            "template",
            envelope,
            existing["tags"],
            _ASSET_FRAME_VERSION,
            payload,
        )
        self._refresh()
        return self._project._bound_template(new_name)

    def delete(self, name: str) -> None:
        self._refresh()
        resolved = _resolve_collection_name(name, self._names, prefix=self._prefix)
        if resolved is None:
            raise KeyError(name)
        if not self._project.geodata.delete_asset(_template_physical_name(resolved)):
            raise KeyError(resolved)
        self._refresh()


class Project:
    """Canonical Python project facade, thin over `GeoData`."""

    def __init__(
        self,
        geo: GeoData,
        *,
        source: str | Path | None = None,
        aliases: Mapping[str, Any] | None = None,
        crs: str | None = None,
        display_name: str | None = None,
        settings: Mapping[str, Any] | None = None,
        inventory: Mapping[str, Any] | None = None,
        tops_tables: Mapping[str, list[dict[str, Any]]] | None = None,
    ) -> None:
        self._geo = geo
        self.source = None if source is None else str(source)
        self.aliases = dict(aliases or {})
        if crs is not None:
            self._geo.crs = crs
        if display_name is not None:
            self._geo.display_name = display_name
        self.settings = dict(settings or {})
        self._inventory = dict(inventory or self._empty_inventory(self.source))
        self._tops_tables = {
            str(name): [dict(row) for row in rows]
            for name, rows in (tops_tables or {}).items()
        }
        self._log_resolution_cache: dict[str, list[dict[str, Any]]] = {}

    @property
    def display_name(self) -> str | None:
        return self._geo.display_name

    @display_name.setter
    def display_name(self, value: str | None) -> None:
        self._geo.display_name = value

    @property
    def crs(self) -> str | None:
        return self._geo.crs

    @crs.setter
    def crs(self, value: str | None) -> None:
        self._geo.crs = value

    @property
    def unit(self) -> str:
        return self._geo.unit

    @classmethod
    def load(cls, path: str | Path) -> "Project":
        """Load a compact `.pproj` project.

        Raw Petrel-style source trees are imported with
        [`Project.import_data`](Project.import_data); `load` is intentionally
        reserved for the compact save/load format.
        """

        src = Path(path)
        if src.suffix.lower() != ".pproj":
            raise ValueError(
                f"Project.load: expected a compact .pproj file, got '{src}'. "
                "Use Project.import_data(...) for raw source folders/files."
            )
        geo = GeoData.open(str(src))
        return cls(
            geo,
            source=src,
            inventory=cls._inventory_from_pproj(src),
            tops_tables={},
        )

    @classmethod
    def import_data(
        cls,
        path: str | Path,
        aliases: Mapping[str, Any] | None = None,
        crs: str | None = None,
        settings: Mapping[str, Any] | ImportSettings | None = None,
        display_name: str | None = None,
    ) -> "Project":
        """Import a raw Petrel-style source directory into a `Project`."""

        src = Path(path)
        aliases, crs, settings_dict = _coerce_import_settings(
            aliases=aliases,
            crs=crs,
            settings=settings,
        )
        if not src.is_dir():
            raise ValueError(
                f"Project.import_data: expected a raw source directory, got '{src}'"
            )

        unit = settings_dict.get("unit", settings_dict.get("units", "m"))
        geo = GeoData(unit=unit)
        alias_map = _normalise_aliases(aliases)
        ingest = IngestSpec(aliases=alias_map, unit=unit) if alias_map else None

        records, sidecars, skipped = _scan(src)
        inventory = cls._empty_inventory(str(src))
        inventory["crs"] = crs
        inventory["aliases"] = dict(aliases or {})
        inventory["sidecars"] = [str(p.relative_to(src)) for p in sidecars]

        loaded_well_dirs = _load_wells(geo, src, records, ingest, settings_dict, inventory, skipped)
        tops_tables = _load_petrel_tops(geo, src, records, inventory, skipped)
        _load_surfaces_points_polygons(
            geo,
            src,
            records,
            inventory,
            skipped,
            ignored_dirs=loaded_well_dirs,
        )
        inventory["counts"] = {
            "surfaces": len(inventory["surfaces"]),
            "wells": len(inventory["wells"]),
            "tops": len(inventory["tops"]),
            "points": len(inventory["points"]),
            "polygons": len(inventory["polygons"]),
            "skipped": len(skipped),
        }
        inventory["skipped"] = skipped
        return cls(
            geo,
            source=src,
            aliases=aliases,
            crs=crs,
            display_name=display_name,
            settings=settings_dict,
            inventory=inventory,
            tops_tables=tops_tables,
        )

    def save(self, path: str | Path) -> None:
        """Save this project to the compact `.pproj` format."""

        dst = Path(path)
        if dst.suffix.lower() != ".pproj":
            raise ValueError(f"Project.save: expected a .pproj path, got '{dst}'")
        self._geo.save(str(dst))

    def view(
        self,
        selection: Any = None,
        *,
        visible: Any = None,
        property: str | Mapping[str, str] | None = None,
        logs: Any = None,
        template: Any = None,
        tab: str = "auto",
        lod: bool | tuple[int, ...] = True,
        settings: Any = None,
    ) -> Any:
        """Open a lazy, folder-aware multi-view workspace for this project.

        Catalog construction reads metadata only. Surface values, trajectories,
        well tops, and explicitly requested logs are materialized on first
        enable and cached by the optional petekTools workspace.
        """

        from ._project_view import project_view

        return project_view(
            self,
            selection,
            visible=visible,
            property=property,
            logs=logs,
            template=template,
            tab=tab,
            lod=lod,
            settings=settings,
        )

    def view_catalog(self) -> dict[str, Any]:
        """The generic petekTools workspace-v2 catalog (metadata only)."""

        from ._project_view import ProjectViewProvider

        return ProjectViewProvider(self).view_catalog()

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
        """Materialize one transitional workspace-v2 provider resource."""

        from ._project_view import ProjectViewProvider

        return ProjectViewProvider(self).view_resource(
            item_id=item_id,
            view=view,
            lane=lane,
            detail=detail,
            attribute=attribute,
            color_by=color_by,
        )

    @property
    def geodata(self) -> GeoData:
        """The underlying `GeoData` substrate."""

        return self._geo

    @property
    def geo(self) -> GeoData:
        """Short alias for the underlying `GeoData` substrate."""

        return self._geo

    @property
    def surfaces(self) -> _NamedCollection:
        return _NamedCollection(self._inventory.get("surfaces", []), self._geo.surface)

    @property
    def structures(self) -> _NamedCollection:
        """Alias for structural surfaces, with the same folder-aware view."""

        return self.surfaces

    @property
    def points(self) -> _NamedCollection:
        return _NamedCollection(self._inventory.get("points", []), self._geo.points)

    @property
    def polygons(self) -> _NamedCollection:
        return _NamedCollection(self._inventory.get("polygons", []), self._geo.polygons)

    @property
    def wells(self) -> Any:
        return _ProjectWellsView(self)

    @property
    def tops(self) -> _TopsCollection:
        """Loaded well-top set names; index by set name for a pandas DataFrame."""

        return _TopsCollection(self._inventory.get("tops", []), self._tops_tables)

    @property
    def well_tops(self) -> _WellTopsMapping:
        """Mutable persisted formation horizons aggregated from actual bores.

        ``project.tops`` remains the compatible imported source-table view;
        this mapping is reconstructed from serialized ``Top {name, md}`` records.
        """

        return _WellTopsMapping(self)

    @property
    def templates(self) -> _TemplatesCollection:
        """Folder-aware persistent correlation-template library."""

        return _TemplatesCollection(self)

    @property
    def logs(self) -> Any:
        """Compatibility alias for ``project.wells.logs``."""

        return self.wells.logs

    def surface(self, name: str) -> Any:
        return self._geo.surface(name)

    def point_set(self, name: str) -> Any:
        return self._geo.points(name)

    def polygon_set(self, name: str) -> Any:
        return self._geo.polygons(name)

    def well(self, well_id: str) -> Any:
        return self._geo.well(well_id)

    def _bound_template(self, logical_name: str) -> BoundTemplate:
        name = _validate_template_name(logical_name)
        physical = _template_physical_name(name)
        asset = self._geo.asset(physical)
        if asset is None:
            raise KeyError(name)
        if asset["version"] != _ASSET_FRAME_VERSION:
            raise ValueError(
                f"template '{name}' uses unsupported asset frame v{asset['version']}"
            )
        try:
            envelope = json.loads(bytes(asset["envelope"]).decode("utf-8"))
        except (UnicodeDecodeError, json.JSONDecodeError) as exc:
            raise ValueError(f"template '{name}' has an invalid asset envelope") from exc
        if not isinstance(envelope, dict) or envelope.get("asset_type") != "template":
            raise ValueError(f"asset '{physical}' is not a template")
        if envelope.get("codec") != _TEMPLATE_CODEC:
            raise ValueError(
                f"template '{name}' uses unsupported codec {envelope.get('codec')!r}"
            )
        provider = envelope.get("provider")
        if not isinstance(provider, str) or not provider:
            raise ValueError(f"template '{name}' has no provider")
        payload = bytes(asset["bytes"])
        data = _decode_template_payload(payload, name=name)
        intrinsic = _validate_template_name(data.get("name"))
        if intrinsic != name:
            raise ValueError(
                f"template asset '{name}' contains intrinsic name '{intrinsic}'"
            )
        kind = data.get("spec")
        schema_version = data.get("schema_version")
        _validate_template_header(kind, schema_version)
        if envelope.get("schema_version") != schema_version:
            raise ValueError(
                f"template '{name}' envelope/payload schema versions disagree"
            )
        return BoundTemplate(
            self,
            name,
            kind,
            schema_version,
            provider,
            payload,
        )

    def rename(self, kind: str, old: str, new: str) -> None:
        """Rename a project object of `kind` to `new`.

        `kind` accepts singular/plural forms: surface(s), point(s),
        polygon(s), well(s), and top(s).
        """

        self._rename_object(_project_kind_key(kind), old, new)

    def delete(self, kind: str, name: str) -> None:
        """Delete a project object of `kind`."""

        self._delete_object(_project_kind_key(kind), name)

    def rename_surface(self, old: str, new: str) -> None:
        self._rename_object("surfaces", old, new)

    def replace_surface(self, name: str, surface: Any) -> None:
        """Write a detached edited surface back under an existing project name.

        Project-backed handles remain copy-on-write; this explicit call is the
        only mutation boundary and requires unchanged geometry/topology.
        """

        resolved = _resolve_collection_name(name, self._inventory.get("surfaces", []))
        if resolved is None:
            raise KeyError(name)
        self._geo.replace_surface(resolved, surface)

    def delete_surface(self, name: str) -> None:
        self._delete_object("surfaces", name)

    def rename_points(self, old: str, new: str) -> None:
        self._rename_object("points", old, new)

    def delete_points(self, name: str) -> None:
        self._delete_object("points", name)

    def rename_polygons(self, old: str, new: str) -> None:
        self._rename_object("polygons", old, new)

    def delete_polygons(self, name: str) -> None:
        self._delete_object("polygons", name)

    def rename_well(self, old: str, new: str) -> None:
        self._rename_object("wells", old, new)

    def delete_well(self, name: str) -> None:
        self._delete_object("wells", name)

    def rename_tops(self, old: str, new: str) -> None:
        self._rename_object("tops", old, new)

    def delete_tops(self, name: str) -> None:
        self._delete_object("tops", name)

    def _rename_object(self, key: str, old: str, new: str) -> None:
        names = list(self._inventory.get(key, []))
        resolved = _resolve_collection_name(old, names)
        if resolved is None:
            raise KeyError(old)
        new_name = _normalise_project_path(new)
        if not new_name:
            raise ValueError("new project object name cannot be empty")
        if new_name != resolved and new_name in names:
            raise ValueError(f"{key[:-1]} '{new_name}' already exists")

        if key == "surfaces":
            self._geo.rename_surface(resolved, new_name)
        elif key == "points":
            self._geo.rename_points(resolved, new_name)
        elif key == "polygons":
            self._geo.rename_polygons(resolved, new_name)
        elif key == "wells":
            self._geo.rename_well(resolved, new_name)
            self._log_resolution_cache.clear()
        elif key == "tops":
            self._tops_tables[new_name] = self._tops_tables.pop(resolved, [])
        else:
            raise ValueError(f"unsupported project object kind {key!r}")

        self._replace_inventory_name(key, resolved, new_name)

    def _delete_object(self, key: str, name: str) -> None:
        names = list(self._inventory.get(key, []))
        resolved = _resolve_collection_name(name, names)
        if resolved is None:
            raise KeyError(name)

        if key == "surfaces":
            removed = self._geo.delete_surface(resolved)
        elif key == "points":
            removed = self._geo.delete_points(resolved)
        elif key == "polygons":
            removed = self._geo.delete_polygons(resolved)
        elif key == "wells":
            removed = self._geo.delete_well(resolved)
            self._log_resolution_cache.clear()
        elif key == "tops":
            removed = resolved in self._tops_tables or resolved in names
            self._tops_tables.pop(resolved, None)
        else:
            raise ValueError(f"unsupported project object kind {key!r}")
        if not removed:
            raise KeyError(resolved)
        self._remove_inventory_name(key, resolved)

    def _replace_inventory_name(self, key: str, old: str, new: str) -> None:
        items = list(self._inventory.get(key, []))
        self._inventory[key] = [new if item == old else item for item in items]

    def _remove_inventory_name(self, key: str, name: str) -> None:
        self._inventory[key] = [item for item in self._inventory.get(key, []) if item != name]
        counts = dict(self._inventory.get("counts", {}))
        counts[key] = len(self._inventory[key])
        self._inventory["counts"] = counts

    def inventory(self) -> dict[str, Any]:
        """Return a notebook-friendly inventory with counts, names, and skips."""

        inv = dict(self._inventory)
        inv["counts"] = dict(self._inventory.get("counts", {}))
        inv["skipped"] = [dict(item) for item in self._inventory.get("skipped", [])]
        for key in (
            "surfaces",
            "wells",
            "tops",
            "points",
            "polygons",
            "sidecars",
        ):
            inv[key] = list(self._inventory.get(key, []))
        if "templates" in self._inventory:
            inv["templates"] = list(self._inventory["templates"])
        return inv

    def resolve_log_expression(self, source: Any) -> list[dict[str, Any]]:
        """Resolve a lazy log expression to positioned well-log dictionaries.

        The returned shape is intentionally plain Python data so petekIO stays
        independent of petekStatic: each item carries ``x``, ``y`` and
        ``samples=[(depth_m, value), ...]``. Downstream property recipes coerce
        this into their own ``WellLogSpec`` type.
        """

        return self._resolve_positioned_logs(source)

    def resolve_well_logs(self, source: Any) -> list[dict[str, Any]]:
        """Alias used by downstream recipe lowerers."""

        return self._resolve_positioned_logs(source)

    def resolve_log_source(self, source: Mapping[str, Any]) -> list[dict[str, Any]]:
        """Resolve a serialized log-channel source dictionary."""

        return self._resolve_positioned_logs(source)

    def _resolve_positioned_logs(self, source: Any) -> list[dict[str, Any]]:
        from ._logs import LogChannel, LogPredicate

        cache_key = _log_source_cache_key(source)
        cached = self._log_resolution_cache.get(cache_key)
        if cached is not None:
            return _copy_positioned_logs(cached)

        logs = self.wells.logs
        channel = logs.validate(_coerce_log_channel(source, logs))
        if not isinstance(channel, LogChannel):
            raise TypeError("log source must resolve to a LogChannel")

        wells: list[dict[str, Any]] = []
        for well_id in self._inventory.get("wells", []):
            well = self.well(well_id)
            if well is None:
                continue
            for bore in _call_names(well, "bores"):
                sidetrack = well.sidetrack(bore)
                if sidetrack is None:
                    continue
                log = sidetrack.log(channel.mnemonic)
                if log is None:
                    continue
                samples: list[tuple[float, float]] = []
                well_xy: tuple[float, float] | None = None
                for md, value in zip(log.md(), log.values(), strict=True):
                    value_f = _finite_or_none(value)
                    if value_f is None:
                        continue
                    if channel.filter is not None and not _eval_log_predicate(
                        channel.filter,
                        sidetrack,
                        md,
                    ):
                        continue
                    tvd = _finite_or_none(sidetrack.tvd(md))
                    pos = sidetrack.xyz(md)
                    xy = _xy_tuple(pos)
                    if tvd is None or xy is None:
                        continue
                    if well_xy is None:
                        well_xy = xy
                    samples.append((tvd, value_f))
                if samples and well_xy is not None:
                    x, y = well_xy
                    wells.append(
                        {
                            "well_id": _bore_qualified_id(str(well_id), str(bore)),
                            "x": x,
                            "y": y,
                            "samples": samples,
                        }
                    )
        self._log_resolution_cache[cache_key] = _copy_positioned_logs(wells)
        return _copy_positioned_logs(wells)

    def __getattr__(self, name: str) -> Any:
        return getattr(self._geo, name)

    @staticmethod
    def _empty_inventory(source: str | None) -> dict[str, Any]:
        return {
            "source": source,
            "crs": None,
            "aliases": {},
            "surfaces": [],
            "wells": [],
            "tops": [],
            "points": [],
            "polygons": [],
            "sidecars": [],
            "skipped": [],
            "counts": {
                "surfaces": 0,
                "wells": 0,
                "tops": 0,
                "points": 0,
                "polygons": 0,
                "skipped": 0,
            },
        }

    @staticmethod
    def _inventory_from_pproj(path: Path) -> dict[str, Any]:
        inv = Project._empty_inventory(str(path))
        try:
            info = GeoData.inspect(str(path))
        except Exception:
            return inv
        inv["crs"] = info.get("crs")
        for kind, name in info.get("elements", []):
            if kind in {"surface", "structured_mesh", "tri_surface"}:
                inv["surfaces"].append(name)
            elif kind == "well":
                inv["wells"].append(name)
            elif kind == "points":
                inv["points"].append(name)
            elif kind == "polygons":
                inv["polygons"].append(name)
            elif kind == "asset" and name.startswith(_TEMPLATE_ASSET_PREFIX):
                inv.setdefault("templates", []).append(name[len(_TEMPLATE_ASSET_PREFIX) :])
        inv["counts"] = {
            "surfaces": len(inv["surfaces"]),
            "wells": len(inv["wells"]),
            "tops": 0,
            "points": len(inv["points"]),
            "polygons": len(inv["polygons"]),
            "skipped": 0,
        }
        if "templates" in inv:
            inv["counts"]["templates"] = len(inv["templates"])
        return inv


def _log_source_cache_key(source: Any) -> str:
    if isinstance(source, Mapping):
        data = source
    else:
        to_dict = getattr(source, "to_dict", None)
        if callable(to_dict):
            data = to_dict()
        else:
            data = repr(source)
    return json.dumps(data, sort_keys=True, separators=(",", ":"), default=repr)


@dataclass
class AssignLogResult:
    """Result report returned by ``project.wells.assign_log``."""

    created: list[dict[str, Any]]
    skipped: list[dict[str, Any]]
    failed: list[dict[str, Any]]

    def summary(self) -> dict[str, int]:
        return {
            "created": len(self.created),
            "skipped": len(self.skipped),
            "failed": len(self.failed),
        }

    def to_dataframe(self) -> Any:
        import pandas as pd

        rows = (
            [{"status": "created", **row} for row in self.created]
            + [{"status": "skipped", **row} for row in self.skipped]
            + [{"status": "failed", **row} for row in self.failed]
        )
        return pd.DataFrame(rows)

    def __repr__(self) -> str:
        s = self.summary()
        return f"AssignLogResult(created={s['created']}, skipped={s['skipped']}, failed={s['failed']})"


@dataclass
class _LogSeries:
    md: list[float]
    values: list[float]


class _MissingLog(Exception):
    def __init__(self, mnemonic: str) -> None:
        super().__init__(f"missing log '{mnemonic}'")
        self.mnemonic = mnemonic


def _clean_log_name(name: str) -> str:
    if not isinstance(name, str):
        raise TypeError(f"log name must be a string, got {type(name).__name__}")
    return name.strip()


def _eval_log_operand(
    operand: Any,
    sidetrack: Any,
    *,
    target_md: list[float] | None,
    default_interpolation: str,
) -> _LogSeries:
    from ._logs import LogBasis, LogChannel, LogExpression

    if isinstance(operand, (int, float)) and not isinstance(operand, bool):
        if target_md is None:
            raise ValueError("scalar-only log expressions need an explicit basis")
        return _LogSeries(list(target_md), [float(operand)] * len(target_md))

    if isinstance(operand, LogChannel):
        if operand.filter is not None:
            raise ValueError("log arithmetic does not yet support filtered log channels")
        series = _read_log_series(sidetrack, operand.mnemonic)
        if target_md is not None and not _same_md(series.md, target_md):
            series = _resample_series(series, target_md, default_interpolation)
        return series

    if isinstance(operand, LogBasis):
        basis_series = _eval_log_operand(
            operand.basis,
            sidetrack,
            target_md=None,
            default_interpolation=default_interpolation,
        )
        source = _read_log_series(sidetrack, operand.source.mnemonic)
        return _resample_series(source, basis_series.md, operand.interpolation)

    if isinstance(operand, LogExpression):
        left = _eval_log_operand(
            operand.operands[0],
            sidetrack,
            target_md=target_md,
            default_interpolation=default_interpolation,
        )
        right = _eval_log_operand(
            operand.operands[1],
            sidetrack,
            target_md=target_md,
            default_interpolation=default_interpolation,
        )
        if target_md is not None and not _same_md(left.md, target_md):
            left = _resample_series(left, target_md, default_interpolation)
        if target_md is not None and not _same_md(right.md, target_md):
            right = _resample_series(right, target_md, default_interpolation)
        if not _same_md(left.md, right.md):
            raise ValueError(
                "log basis mismatch: operands have different MD sampling; "
                "pass basis=logs.<curve> or use .to_basis(logs.<curve>, interpolation=...)"
            )
        return _LogSeries(
            left.md,
            [_apply_log_op(operand.op, a, b) for a, b in zip(left.values, right.values, strict=True)],
        )

    raise TypeError(f"unsupported log expression operand {type(operand).__name__}")


def _read_log_series(sidetrack: Any, mnemonic: str) -> _LogSeries:
    log = sidetrack.log(mnemonic)
    if log is None:
        raise _MissingLog(mnemonic)
    values, md = log.values_md()
    return _LogSeries(list(md), list(values))


def _same_md(a: list[float], b: list[float]) -> bool:
    return len(a) == len(b) and all(x == y for x, y in zip(a, b, strict=True))


def _apply_log_op(op: str, a: float, b: float) -> float:
    if op == "+":
        return a + b
    if op == "-":
        return a - b
    if op == "*":
        return a * b
    if op == "/":
        return a / b
    raise ValueError(f"unsupported log arithmetic operator {op!r}")


def _resample_series(series: _LogSeries, target_md: list[float], interpolation: str) -> _LogSeries:
    try:
        import petektools as _pt  # type: ignore
    except Exception:
        _pt = None
    if _pt is not None and hasattr(_pt, "interp1d"):
        values = list(_pt.interp1d(series.md, series.values, target_md, interpolation))
        return _LogSeries(list(target_md), values)
    values = [_interp_at(series.md, series.values, md, interpolation) for md in target_md]
    return _LogSeries(list(target_md), values)


def _interp_at(md: list[float], values: list[float], x: float, method: str) -> float:
    if not md or x < md[0] or x > md[-1] or math.isnan(x):
        return math.nan
    import bisect

    i = bisect.bisect_left(md, x)
    if i < len(md) and md[i] == x:
        return values[i]
    if method == "nearest":
        if i <= 0:
            return values[0]
        if i >= len(md):
            return values[-1]
        return values[i - 1] if abs(x - md[i - 1]) <= abs(md[i] - x) else values[i]
    if method == "previous":
        return values[max(0, i - 1)]
    if method == "next":
        return values[min(i, len(values) - 1)]
    if i <= 0 or i >= len(md):
        return math.nan
    if method == "linear":
        return _linear_interp(md[i - 1], values[i - 1], md[i], values[i], x)
    if method == "spline":
        return _cubic_interp(md, values, i, x)
    raise ValueError(f"unsupported interpolation {method!r}")


def _linear_interp(x0: float, y0: float, x1: float, y1: float, x: float) -> float:
    span = x1 - x0
    if span <= 0:
        return y0
    t = (x - x0) / span
    return y0 + (y1 - y0) * t


def _cubic_interp(md: list[float], values: list[float], i: int, x: float) -> float:
    if i < 2 or i + 1 >= len(md):
        return _linear_interp(md[i - 1], values[i - 1], md[i], values[i], x)
    x0, x1, x2, x3 = md[i - 2], md[i - 1], md[i], md[i + 1]
    y0, y1, y2, y3 = values[i - 2], values[i - 1], values[i], values[i + 1]
    if x2 == x1:
        return y1
    t = (x - x1) / (x2 - x1)
    m1 = (y2 - y0) / (x2 - x0) * (x2 - x1) if x2 != x0 else y2 - y1
    m2 = (y3 - y1) / (x3 - x1) * (x2 - x1) if x3 != x1 else y2 - y1
    h00 = 2 * t**3 - 3 * t**2 + 1
    h10 = t**3 - 2 * t**2 + t
    h01 = -2 * t**3 + 3 * t**2
    h11 = t**3 - t**2
    return h00 * y1 + h10 * m1 + h01 * y2 + h11 * m2


def _copy_positioned_logs(wells: list[dict[str, Any]]) -> list[dict[str, Any]]:
    return [
        {
            **{key: value for key, value in well.items() if key != "samples"},
            "samples": list(well.get("samples", [])),
        }
        for well in wells
    ]


def _coerce_import_settings(
    *,
    aliases: Mapping[str, Any] | None,
    crs: str | None,
    settings: Mapping[str, Any] | ImportSettings | None,
) -> tuple[Mapping[str, Any] | None, str | None, dict[str, Any]]:
    settings_aliases: Mapping[str, Any] | None = None
    settings_crs: str | None = None

    if isinstance(settings, ImportSettings):
        settings_dict = settings.to_mapping()
        settings_aliases = settings.aliases
        settings_crs = settings.crs
    elif settings is None:
        settings_dict = {}
    elif isinstance(settings, Mapping):
        settings_dict = dict(settings)
        raw_aliases = settings_dict.pop("aliases", None)
        if raw_aliases is not None:
            if not isinstance(raw_aliases, Mapping):
                raise TypeError("Project.import_data settings['aliases'] must be a mapping")
            settings_aliases = raw_aliases
        raw_crs = settings_dict.pop("crs", None)
        if raw_crs is not None:
            settings_crs = str(raw_crs)
    else:
        raise TypeError("Project.import_data settings must be a mapping or ImportSettings")

    return (
        aliases if aliases is not None else settings_aliases,
        crs if crs is not None else settings_crs,
        settings_dict,
    )


def _normalise_aliases(aliases: Mapping[str, Any] | None) -> dict[str, str]:
    if not aliases:
        return {}
    out: dict[str, str] = {}
    for key, value in aliases.items():
        if isinstance(value, str):
            out[str(key)] = value
        elif isinstance(value, Iterable):
            canonical = str(key)
            for raw in value:
                out[str(raw)] = canonical
        else:
            raise TypeError(
                "Project.import_data aliases values must be strings or iterables of strings"
            )
    return out


def _project_kind_key(kind: str) -> str:
    token = _lookup_token(kind)
    aliases = {
        "surface": "surfaces",
        "surfaces": "surfaces",
        "structure": "surfaces",
        "structures": "surfaces",
        "point": "points",
        "points": "points",
        "pointset": "points",
        "pointsets": "points",
        "polygon": "polygons",
        "polygons": "polygons",
        "polygonset": "polygons",
        "polygonsets": "polygons",
        "well": "wells",
        "wells": "wells",
        "top": "tops",
        "tops": "tops",
        "topset": "tops",
        "topsets": "tops",
    }
    try:
        return aliases[token]
    except KeyError as exc:
        raise ValueError(f"unsupported project object kind {kind!r}") from exc


def _canonical_json_bytes(value: Any, *, what: str) -> bytes:
    try:
        text = json.dumps(
            value,
            allow_nan=False,
            ensure_ascii=False,
            separators=(",", ":"),
            sort_keys=True,
        )
    except (TypeError, ValueError) as exc:
        raise ValueError(f"{what} must contain only finite JSON data: {exc}") from exc
    return text.encode("utf-8")


def _validate_template_name(value: Any) -> str:
    if not isinstance(value, str):
        raise TypeError("template name must be a string")
    if not value or value != value.strip() or len(value.encode("utf-8")) > 900:
        raise ValueError("template name must be non-empty, trimmed, and at most 900 UTF-8 bytes")
    if value.startswith("/") or value.endswith("/") or "\\" in value or "\0" in value:
        raise ValueError(f"invalid template name {value!r}")
    parts = value.split("/")
    if any(not part or part in {".", ".."} for part in parts) or parts[0] == "@asset":
        raise ValueError(
            "template names may use folders but not empty, reserved, or traversal segments"
        )
    return value


def _validate_template_header(kind: Any, schema_version: Any) -> tuple[str, int]:
    if not isinstance(kind, str) or re.fullmatch(r"[A-Za-z][A-Za-z0-9_]*", kind) is None:
        raise ValueError("template 'spec' must be a provider type name")
    if (
        isinstance(schema_version, bool)
        or not isinstance(schema_version, int)
        or schema_version <= 0
        or schema_version > 2**32 - 1
    ):
        raise ValueError("template 'schema_version' must be a positive 32-bit integer")
    return kind, schema_version


def _template_snapshot(
    template: Any,
) -> tuple[str, str, int, str, bytes, bytes]:
    if isinstance(template, Mapping):
        raw = template
    else:
        to_dict = getattr(template, "to_dict", None)
        if not callable(to_dict):
            raise TypeError("template must be a mapping or expose to_dict()")
        raw = to_dict()
    if not isinstance(raw, Mapping):
        raise TypeError("template.to_dict() must return a mapping")
    data = dict(raw)
    name = _validate_template_name(data.get("name"))
    kind, schema_version = _validate_template_header(
        data.get("spec"), data.get("schema_version")
    )
    payload = _canonical_json_bytes(data, what=f"template '{name}'")
    # Decode once to prove the snapshot is a JSON object rather than relying on
    # a Mapping implementation with surprising conversion behaviour.
    _decode_template_payload(payload, name=name)
    provider = f"petektools.viewer.{kind}"
    envelope = _canonical_json_bytes(
        {
            "asset_type": "template",
            "codec": _TEMPLATE_CODEC,
            "provider": provider,
            "schema_version": schema_version,
        },
        what="template asset envelope",
    )
    return name, kind, schema_version, provider, envelope, payload


def _decode_template_payload(payload: bytes, *, name: str) -> dict[str, Any]:
    try:
        data = json.loads(payload.decode("utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError) as exc:
        raise ValueError(f"template '{name}' payload is not valid UTF-8 JSON") from exc
    if not isinstance(data, dict):
        raise ValueError(f"template '{name}' payload must be a JSON object")
    canonical = _canonical_json_bytes(data, what=f"template '{name}'")
    if canonical != payload:
        raise ValueError(f"template '{name}' payload is not canonical JSON")
    return data


def _template_physical_name(logical_name: str) -> str:
    return f"{_TEMPLATE_ASSET_PREFIX}{_validate_template_name(logical_name)}"


def _template_provider_guidance(kind: str) -> str:
    return (
        f"rendering/materializing {kind} requires a compatible petektools.viewer. "
        "Install or upgrade it with `pip install -U 'petekio[toolkit]'`. "
        "Project template listing and .pproj persistence work without petektools."
    )


def _well_attr_lookup(attr: str, well_ids: Iterable[str]) -> str | None:
    for well_id in well_ids:
        if attr == well_id or attr == _identifier_for(well_id):
            return well_id
    return None


def _normalise_project_path(name: str | Path) -> str:
    parts = [part.strip() for part in str(name).replace("\\", "/").split("/") if part.strip()]
    return "/".join(parts)


def _join_project_path(prefix: str, name: str) -> str:
    prefix = _normalise_project_path(prefix)
    name = _normalise_project_path(name)
    if not prefix:
        return name
    if not name:
        return prefix
    return f"{prefix}/{name}"


def _relative_to_project_prefix(name: str, prefix: str) -> str | None:
    name = _normalise_project_path(name)
    prefix = _normalise_project_path(prefix)
    if not prefix:
        return name
    marker = f"{prefix}/"
    if name.startswith(marker):
        return name[len(marker) :]
    return None


def _visible_collection_names(names: Iterable[str], prefix: str = "") -> list[str]:
    out: list[str] = []
    seen: set[str] = set()
    for name in names:
        rel = _relative_to_project_prefix(name, prefix)
        if not rel:
            continue
        child = rel.split("/", 1)[0]
        visible = f"{child}/" if "/" in rel else child
        if visible not in seen:
            seen.add(visible)
            out.append(visible)
    return out


def _resolve_collection_folder(
    name: str,
    names: Iterable[str],
    *,
    prefix: str = "",
) -> str | None:
    token = _lookup_token(name)
    matches: list[str] = []
    for visible in _visible_collection_names(names, prefix):
        if not visible.endswith("/"):
            continue
        folder = visible[:-1]
        if folder == name or _lookup_token(folder) == token:
            matches.append(_join_project_path(prefix, folder))
    return matches[0] if len(matches) == 1 else None


def _resolve_collection_name(
    name: str,
    names: Iterable[str],
    *,
    prefix: str = "",
) -> str | None:
    names_list = [_normalise_project_path(candidate) for candidate in names]
    requested = _normalise_project_path(name)
    scoped = _join_project_path(prefix, requested)
    for candidate in (requested, scoped):
        if candidate in names_list:
            return candidate

    token = _lookup_token(name)
    immediate_matches = []
    for candidate in names_list:
        rel = _relative_to_project_prefix(candidate, prefix)
        if rel is not None and "/" not in rel and _lookup_token(rel) == token:
            immediate_matches.append(candidate)
    if len(immediate_matches) == 1:
        return immediate_matches[0]

    leaf_matches = [
        candidate
        for candidate in names_list
        if _lookup_token(_collection_leaf_name(candidate)) == token
    ]
    if len(leaf_matches) == 1:
        return leaf_matches[0]
    return None


def _collection_leaf_name(name: str) -> str:
    leaf = _normalise_project_path(name).rsplit("/", 1)[-1]
    return leaf.rsplit(".", 1)[-1]


def _lookup_token(value: str) -> str:
    return re.sub(r"[^0-9a-z]+", "", str(value).casefold())


def _identifier_for(name: str) -> str:
    ident = re.sub(r"\W+", "_", str(name)).strip("_")
    if not ident:
        return "_"
    if ident[0].isdigit():
        ident = f"_{ident}"
    return ident


def _well_log_names(well: Any) -> list[str]:
    names: list[str] = []
    bore_names = _call_or_empty(well, "bores")
    if not bore_names:
        names.extend(str(mnemonic) for mnemonic in _call_or_empty(well, "mnemonics"))
    else:
        for bore in bore_names:
            sidetrack = well.sidetrack(bore)
            if sidetrack is None:
                continue
            names.extend(str(mnemonic) for mnemonic in _call_or_empty(sidetrack, "mnemonics"))
    return sorted(set(names), key=str.casefold)


def _call_or_empty(obj: Any, method: str) -> list[Any]:
    fn = getattr(obj, method, None)
    if fn is None:
        return []
    try:
        return list(fn())
    except Exception:
        return []


def _coerce_log_channel(source: Any, logs: Any) -> Any:
    from ._logs import LogChannel

    if isinstance(source, LogChannel):
        return source
    if isinstance(source, Mapping):
        kind = source.get("kind")
        if kind not in {"log", "log_channel"}:
            raise TypeError(f"cannot resolve log source kind {kind!r}")
        channel = logs.channel(str(source.get("requested") or source.get("mnemonic")))
        filter_spec = source.get("filter")
        if filter_spec is not None:
            channel = channel.where(_predicate_from_dict(filter_spec, logs))
        return channel
    raise TypeError(f"cannot resolve log source of type {type(source).__name__}")


def _predicate_from_dict(data: Mapping[str, Any], logs: Any) -> Any:
    if data.get("kind") != "log_predicate":
        raise TypeError("serialized log filter must have kind='log_predicate'")
    operands = [_operand_from_dict(item, logs) for item in data.get("operands", [])]
    op = data.get("op")
    if op == "and":
        return operands[0] & operands[1]
    if op == "or":
        return operands[0] | operands[1]
    if op == "not":
        return ~operands[0]
    if op in {"is_finite", "is_null", "not_null"}:
        return getattr(operands[0], op)()
    if op in {">", ">=", "<", "<=", "==", "!="}:
        return getattr(operands[0], _COMPARISON_METHODS[op])(operands[1])
    raise ValueError(f"unsupported log predicate operator {op!r}")


def _operand_from_dict(data: Any, logs: Any) -> Any:
    if not isinstance(data, Mapping):
        return data
    kind = data.get("kind")
    if kind in {"log", "log_channel"}:
        return logs.channel(str(data.get("requested") or data.get("mnemonic")))
    if kind == "log_predicate":
        return _predicate_from_dict(data, logs)
    if kind in {"scalar", "literal"}:
        return data.get("value")
    raise TypeError(f"unsupported log predicate operand kind {kind!r}")


_COMPARISON_METHODS = {
    ">": "__gt__",
    ">=": "__ge__",
    "<": "__lt__",
    "<=": "__le__",
    "==": "__eq__",
    "!=": "__ne__",
}


def _eval_log_predicate(predicate: Any, sidetrack: Any, md: float) -> bool:
    op = predicate.op
    operands = predicate.operands
    if op == "and":
        return _eval_log_predicate(operands[0], sidetrack, md) and _eval_log_predicate(
            operands[1],
            sidetrack,
            md,
        )
    if op == "or":
        return _eval_log_predicate(operands[0], sidetrack, md) or _eval_log_predicate(
            operands[1],
            sidetrack,
            md,
        )
    if op == "not":
        return not _eval_log_predicate(operands[0], sidetrack, md)
    if op == "is_finite":
        return _eval_log_value(operands[0], sidetrack, md) is not None
    if op == "is_null":
        return _eval_log_raw_value(operands[0], sidetrack, md) is None
    if op == "not_null":
        return _eval_log_raw_value(operands[0], sidetrack, md) is not None

    left = _eval_log_value(operands[0], sidetrack, md)
    right = _eval_log_value(operands[1], sidetrack, md)
    if left is None or right is None:
        return False
    if op == ">":
        return left > right
    if op == ">=":
        return left >= right
    if op == "<":
        return left < right
    if op == "<=":
        return left <= right
    if op == "==":
        return left == right
    if op == "!=":
        return left != right
    raise ValueError(f"unsupported log predicate operator {op!r}")


def _eval_log_value(operand: Any, sidetrack: Any, md: float) -> float | None:
    value = _eval_log_raw_value(operand, sidetrack, md)
    return _finite_or_none(value)


def _eval_log_raw_value(operand: Any, sidetrack: Any, md: float) -> Any:
    from ._logs import LogChannel

    if isinstance(operand, LogChannel):
        log = sidetrack.log(operand.mnemonic)
        return None if log is None else log.at_md(md)
    return operand


def _finite_or_none(value: Any) -> float | None:
    if value is None:
        return None
    try:
        out = float(value)
    except (TypeError, ValueError):
        return None
    return out if isfinite(out) else None


def _xy_tuple(pos: Any) -> tuple[float, float] | None:
    if pos is None:
        return None
    try:
        x, y = float(pos[0]), float(pos[1])
    except (TypeError, ValueError, IndexError):
        return None
    if not isfinite(x) or not isfinite(y):
        return None
    return x, y


def _call_names(obj: Any, method: str) -> list[Any]:
    fn = getattr(obj, method, None)
    if not callable(fn):
        return []
    try:
        return list(fn())
    except Exception:
        return []


def _bore_qualified_id(well_id: str, bore: str) -> str:
    return well_id if not bore else f"{well_id} {bore}"


def _scan(root: Path) -> tuple[list[_FileRecord], list[Path], list[dict[str, str]]]:
    records: list[_FileRecord] = []
    sidecars: list[Path] = []
    skipped: list[dict[str, str]] = []
    for path in sorted(p for p in root.rglob("*") if p.is_file()):
        try:
            kind = detect(str(path))
        except Exception as exc:
            skipped.append(_skip(root, path, SKIP_IMPORT_ERROR, str(exc)))
            continue
        if kind == FormatKind.CrsMetaXml:
            sidecars.append(path)
        records.append(_FileRecord(path, kind))
    return records, sidecars, skipped


def _load_wells(
    geo: GeoData,
    root: Path,
    records: list[_FileRecord],
    ingest: IngestSpec | None,
    settings: Mapping[str, Any],
    inventory: dict[str, Any],
    skipped: list[dict[str, str]],
) -> set[Path]:
    well_records = [
        r for r in records if r.kind in (FormatKind.Las, FormatKind.WellPath, FormatKind.CrsMetaXml)
    ]
    groups: dict[str, list[Path]] = {}
    for rec in well_records:
        if rec.kind == FormatKind.CrsMetaXml:
            continue
        well_id = _infer_well_id(rec.path, root)
        groups.setdefault(well_id, []).append(rec.path)

    loaded_dirs: set[Path] = set()
    for well_id in sorted(groups):
        files = _well_load_root(well_id, root, groups[well_id])
        kwargs: dict[str, Any] = {"files": str(files)}
        if "head" in settings:
            kwargs["head"] = settings["head"]
        if "kb" in settings:
            kwargs["kb"] = settings["kb"]
        if ingest is not None:
            kwargs["ingest"] = ingest
        try:
            geo.load_well(well_id, **kwargs)
        except Exception as exc:
            skipped.append(_skip(root, files, SKIP_IMPORT_ERROR, f"load_well {well_id}: {exc}"))
            continue
        inventory["wells"].append(well_id)
        if files.is_dir() and files != root:
            loaded_dirs.add(files)
    return loaded_dirs


def _load_petrel_tops(
    geo: GeoData,
    root: Path,
    records: list[_FileRecord],
    inventory: dict[str, Any],
    skipped: list[dict[str, str]],
) -> dict[str, list[dict[str, Any]]]:
    top_records = [rec for rec in records if rec.kind == FormatKind.PetrelTops]
    stem_counts: dict[str, int] = {}
    for rec in top_records:
        stem_counts[rec.path.stem] = stem_counts.get(rec.path.stem, 0) + 1
    tables: dict[str, list[dict[str, Any]]] = {}
    for rec in top_records:
        name = rec.path.stem if stem_counts.get(rec.path.stem, 0) <= 1 else _name_for(root, rec.path)
        try:
            geo.load_well_tops(str(rec.path))
        except Exception as exc:
            skipped.append(_skip(root, rec.path, SKIP_IMPORT_ERROR, f"load_well_tops: {exc}"))
            continue
        inventory["tops"].append(name)
        try:
            tables[name] = _parse_petrel_tops_table(rec.path)
        except Exception as exc:
            skipped.append(_skip(root, rec.path, SKIP_IMPORT_ERROR, f"parse_well_tops: {exc}"))
            tables[name] = []
    return tables


def _load_surfaces_points_polygons(
    geo: GeoData,
    root: Path,
    records: list[_FileRecord],
    inventory: dict[str, Any],
    skipped: list[dict[str, str]],
    *,
    ignored_dirs: set[Path],
) -> None:
    loaded_paths = {r.path for r in records if r.kind in (FormatKind.Las, FormatKind.WellPath)}
    stem_counts = _spatial_stem_counts(records, ignored_dirs=ignored_dirs, loaded_paths=loaded_paths)
    earthvision_topology = _earthvision_topology_by_stem(
        records,
        ignored_dirs=ignored_dirs,
        loaded_paths=loaded_paths,
    )
    for rec in records:
        if rec.path in loaded_paths or rec.kind in (FormatKind.CrsMetaXml, FormatKind.PetrelTops):
            continue
        if _inside_any(rec.path, ignored_dirs):
            continue

        role = _spatial_role(rec.path, rec.kind)
        if role is None:
            skipped.append(_skip(root, rec.path, SKIP_UNSUPPORTED_FORMAT, repr(rec.kind)))
            continue
        if role == "ambiguous_geojson":
            skipped.append(_skip(root, rec.path, SKIP_AMBIGUOUS_GEOJSON, repr(rec.kind)))
            continue

        name = _asset_name_for(root, rec.path, role=role, stem_counts=stem_counts)
        try:
            if role == "surface":
                if rec.kind == FormatKind.EarthVisionGrid:
                    geo.load_structured_surface(name, str(rec.path))
                else:
                    geo.load_surface(name, str(rec.path))
                inventory["surfaces"].append(name)
            elif role == "points":
                topology = (
                    earthvision_topology.get(rec.path.stem.casefold())
                    if rec.kind == FormatKind.IrapClassicPoints
                    else None
                )
                if topology is not None:
                    try:
                        geo.load_points_with_topology(name, str(rec.path), str(topology.path))
                    except Exception:
                        geo.load_points(name, str(rec.path))
                else:
                    geo.load_points(name, str(rec.path))
                inventory["points"].append(name)
            elif role == "polygons":
                geo.load_polygons(name, str(rec.path))
                inventory["polygons"].append(name)
        except Exception as exc:
            skipped.append(_skip(root, rec.path, SKIP_IMPORT_ERROR, f"load_{role}: {exc}"))


def _earthvision_topology_by_stem(
    records: list[_FileRecord],
    *,
    ignored_dirs: set[Path],
    loaded_paths: set[Path],
) -> dict[str, _FileRecord]:
    by_stem: dict[str, _FileRecord] = {}
    ambiguous: set[str] = set()
    for rec in records:
        if rec.path in loaded_paths or rec.kind != FormatKind.EarthVisionGrid:
            continue
        if _inside_any(rec.path, ignored_dirs):
            continue
        key = rec.path.stem.casefold()
        if key in by_stem:
            ambiguous.add(key)
            continue
        by_stem[key] = rec
    for key in ambiguous:
        by_stem.pop(key, None)
    return by_stem


def _spatial_stem_counts(
    records: list[_FileRecord],
    *,
    ignored_dirs: set[Path],
    loaded_paths: set[Path],
) -> dict[tuple[str, str], int]:
    counts: dict[tuple[str, str], int] = {}
    for rec in records:
        if rec.path in loaded_paths or rec.kind in (FormatKind.CrsMetaXml, FormatKind.PetrelTops):
            continue
        if _inside_any(rec.path, ignored_dirs):
            continue
        role = _spatial_role(rec.path, rec.kind)
        if role not in {"surface", "points", "polygons"}:
            continue
        key = (role, rec.path.stem)
        counts[key] = counts.get(key, 0) + 1
    # An EarthVision structured surface and its same-stem IRAP point export are
    # distinct roles, but both historically received path-qualified names.
    # Preserve those stable names while allowing both objects to coexist.
    kinds_by_stem: dict[str, list[FormatKind]] = {}
    for rec in records:
        if rec.path in loaded_paths or _inside_any(rec.path, ignored_dirs):
            continue
        kinds_by_stem.setdefault(rec.path.stem, []).append(rec.kind)
    for stem, kinds in kinds_by_stem.items():
        if FormatKind.EarthVisionGrid in kinds and FormatKind.IrapClassicPoints in kinds:
            counts[("surface", stem)] = max(2, counts.get(("surface", stem), 0))
            counts[("points", stem)] = max(2, counts.get(("points", stem), 0))
    return counts


def _spatial_role(path: Path, kind: FormatKind) -> str | None:
    suffix = path.suffix.lower()
    if kind in (FormatKind.IrapClassicGrid, FormatKind.Cps3Grid, FormatKind.EarthVisionGrid):
        return "surface"
    if kind == FormatKind.IrapClassicPoints:
        return "points"
    if kind == FormatKind.CsvPoints:
        return "points" if _csv_has_xyz(path) else None
    if kind == FormatKind.Cps3Lines:
        return "polygons"
    if kind == FormatKind.GeoJson:
        return _geojson_role(path)
    if kind == FormatKind.Unknown:
        if suffix in {".pol", ".cps3lines", ".shp"}:
            return "polygons"
        if suffix in {".xyz", ".dat", ".irapclassicpoints"}:
            return "points"
        if suffix in {".irap", ".gri", ".cps3grid"}:
            return "surface"
    return None


def _csv_has_xyz(path: Path) -> bool:
    try:
        for line in path.read_text(errors="ignore").splitlines():
            if not line.strip() or "," not in line:
                continue
            cols = [c.strip().strip('"').lower() for c in line.split(",")]
            has_x = any(c in {"x", "easting"} for c in cols)
            has_y = any(c in {"y", "northing"} for c in cols)
            has_z = any(c in {"z", "depth", "tvd"} for c in cols)
            return has_x and has_y and has_z
    except OSError:
        return False
    return False


def _geojson_role(path: Path) -> str:
    try:
        text = path.read_text(errors="ignore").lower()
    except OSError:
        return "ambiguous_geojson"
    has_polygon = '"polygon"' in text or '"multipolygon"' in text
    has_point = '"point"' in text or '"multipoint"' in text
    if has_polygon and not has_point:
        return "polygons"
    if has_point and not has_polygon:
        return "points"
    stem = path.stem.lower()
    if any(token in stem for token in ("poly", "outline", "boundary", "fault")):
        return "polygons"
    if any(token in stem for token in ("point", "sample", "welltop")):
        return "points"
    return "ambiguous_geojson"


def _infer_well_id(path: Path, root: Path) -> str:
    stem_id = _id_from_token(path.stem)
    if stem_id:
        return stem_id
    for parent in path.parents:
        if parent == root.parent:
            break
        parent_id = _id_from_token(parent.name)
        if parent_id:
            return parent_id
    return path.stem


def _id_from_token(token: str) -> str | None:
    parts = [p for p in token.replace(" ", "_").split("_") if p]
    if not parts:
        return None
    if len(parts) >= 2 and parts[0].isdigit() and parts[1][:1].isdigit():
        return f"{parts[0]}/{parts[1]}"
    if len(parts) >= 2 and _looks_like_bore(parts[-1]):
        return "_".join(parts[:-1])
    generic = {
        "logs",
        "paths",
        "wells",
        "tops",
        "surfaces",
        "points",
        "polygons",
        "sample",
        "log",
        "core",
        "complogs",
        "comp_logs",
    }
    if token.lower() in generic:
        return None
    return token.replace("_", "/") if token[:2].isdigit() and "_" in token else token


def _looks_like_bore(token: str) -> bool:
    up = token.upper()
    return up in {"A", "B", "C"} or up.startswith("ST") or up.startswith("COMPLOG")


def _well_load_root(well_id: str, root: Path, paths: list[Path]) -> Path:
    if any(_file_stem_matches_id(p, well_id) for p in paths):
        return root
    common = Path(_common_path([str(p.parent) for p in paths]))
    return common if common.is_dir() else paths[0]


def _file_stem_matches_id(path: Path, well_id: str) -> bool:
    return _normal_key(path.stem) == _normal_key(well_id) or _normal_key(path.stem).startswith(
        _normal_key(well_id) + "_"
    )


def _normal_key(value: str) -> str:
    return value.strip().lower().replace("/", "_").replace("-", "_").replace(" ", "_")


def _common_path(paths: list[str]) -> str:
    if not paths:
        return "."
    try:
        import os

        return os.path.commonpath(paths)
    except ValueError:
        return paths[0]


def _inside_any(path: Path, dirs: set[Path]) -> bool:
    for directory in dirs:
        try:
            path.relative_to(directory)
            return True
        except ValueError:
            continue
    return False


def _name_for(root: Path, path: Path) -> str:
    rel = path.relative_to(root)
    return rel.with_suffix("").as_posix()


def _asset_name_for(
    root: Path,
    path: Path,
    *,
    role: str,
    stem_counts: Mapping[tuple[str, str], int],
) -> str:
    stem = path.stem
    if stem_counts.get((role, stem), 0) <= 1:
        return stem
    return _name_for(root, path)


def _parse_petrel_tops_table(path: Path) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    header: list[str] = []
    in_header = False
    for raw in path.read_text(errors="replace").splitlines():
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        upper = line.upper()
        if upper == "BEGIN HEADER":
            in_header = True
            header = []
            continue
        if upper == "END HEADER":
            in_header = False
            continue
        if in_header:
            header.append(_top_column_name(line))
            continue
        if not header:
            continue
        parts = shlex.split(line)
        if len(parts) < len(header):
            continue
        row: dict[str, Any] = {
            key: _coerce_top_value(value)
            for key, value in zip(header, parts, strict=False)
        }
        row["source"] = str(path)
        rows.append(row)
    return rows


def _top_column_name(name: str) -> str:
    key = str(name).strip().casefold().replace(" ", "_")
    aliases = {
        "surface": "surface",
        "well": "well",
        "type": "type",
        "md": "md",
        "pvd": "pvd",
        "twt": "twt",
        "twt2": "twt2",
        "age": "age",
        "x": "x",
        "y": "y",
        "z": "z",
    }
    return aliases.get(key, key)


def _coerce_top_value(value: str) -> Any:
    try:
        return float(value)
    except ValueError:
        return value


def _dataframe(rows: list[dict[str, Any]]) -> Any:
    try:
        import pandas as pd
    except Exception as exc:  # pragma: no cover - optional dependency
        raise ImportError(
            "project.tops[...] requires pandas; install with `pip install petekio[pandas]`"
        ) from exc
    return pd.DataFrame(rows)


def _skip(root: Path, path: Path, reason: str, detail: str) -> dict[str, str]:
    try:
        display = path.relative_to(root).as_posix()
    except ValueError:
        display = str(path)
    return {"path": display, "reason": reason, "detail": detail}
