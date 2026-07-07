"""Project-level raw-tree loading facade for petekIO.

`Project` is intentionally a Python facade over `GeoData`: it owns scan metadata
and user-facing inventory, while all loaded subsurface data remains in the Rust
`GeoData` substrate.
"""

from __future__ import annotations

import json
import re
import shlex
from collections.abc import Iterable, Iterator, Mapping
from dataclasses import dataclass, field
from math import isfinite
from pathlib import Path
from typing import Any

from ._petekio import FormatKind, GeoData, IngestSpec, detect


SKIP_UNSUPPORTED_FORMAT = "unsupported_format"
SKIP_AMBIGUOUS_GEOJSON = "ambiguous_geojson"
SKIP_LOAD_ERROR = "load_error"


@dataclass(frozen=True)
class _FileRecord:
    path: Path
    kind: FormatKind


@dataclass(frozen=True)
class LoadSettings:
    """Project loading settings owned by petekIO.

    ``Project.load(..., settings=LoadSettings(...))`` is the notebook-facing
    form. The explicit ``aliases=``, ``crs=``, and mapping-style ``settings=``
    arguments remain supported for compatibility with earlier examples.
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
    """List-like project names with optional lookup by name."""

    def __init__(self, names: Iterable[str], getter):
        self._names = list(names)
        self._getter = getter

    def __len__(self) -> int:
        return len(self._names)

    def __iter__(self) -> Iterator[str]:
        return iter(self._names)

    def __contains__(self, name: object) -> bool:
        return isinstance(name, str) and _resolve_collection_name(name, self._names) is not None

    def __getitem__(self, name: int | slice | str) -> Any:
        if isinstance(name, (int, slice)):
            return self._names[name]
        resolved = _resolve_collection_name(name, self._names)
        if resolved is None:
            raise KeyError(name)
        value = self._getter(resolved)
        if value is None:
            raise KeyError(resolved)
        return value

    def __call__(self, name: str) -> Any:
        resolved = _resolve_collection_name(name, self._names)
        return None if resolved is None else self._getter(resolved)

    def get(self, name: str, default: Any = None) -> Any:
        resolved = _resolve_collection_name(name, self._names)
        if resolved is None:
            return default
        value = self._getter(resolved)
        return default if value is None else value

    def names(self) -> list[str]:
        return list(self._names)

    def values(self) -> list[Any]:
        return [self._getter(name) for name in self._names]

    def items(self) -> list[tuple[str, Any]]:
        return [(name, self._getter(name)) for name in self._names]

    def __eq__(self, other: object) -> bool:
        if isinstance(other, _NamedCollection):
            return self._names == other._names
        if isinstance(other, list):
            return self._names == other
        return False

    def __repr__(self) -> str:
        return repr(self._names)

    __str__ = __repr__


class _TopsCollection:
    """List-like top-set names with DataFrame lookup by set name."""

    def __init__(self, names: Iterable[str], rows_by_name: Mapping[str, list[dict[str, Any]]]):
        self._names = list(names)
        self._rows_by_name = {str(name): list(rows) for name, rows in rows_by_name.items()}

    def __len__(self) -> int:
        return len(self._names)

    def __iter__(self) -> Iterator[str]:
        return iter(self._names)

    def __contains__(self, name: object) -> bool:
        return isinstance(name, str) and _resolve_collection_name(name, self._names) is not None

    def __getitem__(self, name: int | slice | str) -> Any:
        if isinstance(name, (int, slice)):
            return self._names[name]
        resolved = _resolve_collection_name(name, self._names)
        if resolved is None:
            raise KeyError(name)
        return _dataframe(self._rows_by_name.get(resolved, []))

    def get(self, name: str, default: Any = None) -> Any:
        try:
            return self[name]
        except KeyError:
            return default

    def names(self) -> list[str]:
        return list(self._names)

    def items(self) -> list[tuple[str, Any]]:
        return [(name, self[name]) for name in self._names]

    def __eq__(self, other: object) -> bool:
        if isinstance(other, _TopsCollection):
            return self._names == other._names
        if isinstance(other, list):
            return self._names == other
        return False

    def __repr__(self) -> str:
        return repr(self._names)

    __str__ = __repr__


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


class Project:
    """Canonical Python project-loading facade, thin over `GeoData`."""

    def __init__(
        self,
        geo: GeoData,
        *,
        source: str | Path | None = None,
        aliases: Mapping[str, Any] | None = None,
        crs: str | None = None,
        settings: Mapping[str, Any] | None = None,
        inventory: Mapping[str, Any] | None = None,
        tops_tables: Mapping[str, list[dict[str, Any]]] | None = None,
    ) -> None:
        self._geo = geo
        self.source = None if source is None else str(source)
        self.aliases = dict(aliases or {})
        self.crs = crs
        self.settings = dict(settings or {})
        self._inventory = dict(inventory or self._empty_inventory(self.source))
        self._tops_tables = {
            str(name): [dict(row) for row in rows]
            for name, rows in (tops_tables or {}).items()
        }
        self._log_resolution_cache: dict[str, list[dict[str, Any]]] = {}

    @classmethod
    def load(
        cls,
        path: str | Path,
        aliases: Mapping[str, Any] | None = None,
        crs: str | None = None,
        settings: Mapping[str, Any] | LoadSettings | None = None,
    ) -> "Project":
        """Load a `.pproj` or recursively ingest a raw Petrel-style directory."""

        src = Path(path)
        aliases, crs, settings_dict = _coerce_load_settings(
            aliases=aliases,
            crs=crs,
            settings=settings,
        )
        if src.suffix.lower() == ".pproj":
            geo = GeoData.open(str(src))
            return cls(
                geo,
                source=src,
                aliases=aliases,
                crs=crs,
                settings=settings_dict,
                inventory=cls._inventory_from_pproj(src),
                tops_tables={},
            )
        if not src.is_dir():
            raise ValueError(f"Project.load: expected a .pproj file or directory, got '{src}'")

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
            settings=settings_dict,
            inventory=inventory,
            tops_tables=tops_tables,
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

    def inventory(self) -> dict[str, Any]:
        """Return a notebook-friendly inventory with counts, names, and skips."""

        inv = dict(self._inventory)
        inv["counts"] = dict(self._inventory.get("counts", {}))
        inv["skipped"] = [dict(item) for item in self._inventory.get("skipped", [])]
        for key in ("surfaces", "wells", "tops", "points", "polygons", "sidecars"):
            inv[key] = list(self._inventory.get(key, []))
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
        for kind, name in info.get("elements", []):
            if kind == "surface":
                inv["surfaces"].append(name)
            elif kind == "well":
                inv["wells"].append(name)
            elif kind == "points":
                inv["points"].append(name)
            elif kind == "polygons":
                inv["polygons"].append(name)
        inv["counts"] = {
            "surfaces": len(inv["surfaces"]),
            "wells": len(inv["wells"]),
            "tops": 0,
            "points": len(inv["points"]),
            "polygons": len(inv["polygons"]),
            "skipped": 0,
        }
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


def _copy_positioned_logs(wells: list[dict[str, Any]]) -> list[dict[str, Any]]:
    return [
        {
            **{key: value for key, value in well.items() if key != "samples"},
            "samples": list(well.get("samples", [])),
        }
        for well in wells
    ]


def _coerce_load_settings(
    *,
    aliases: Mapping[str, Any] | None,
    crs: str | None,
    settings: Mapping[str, Any] | LoadSettings | None,
) -> tuple[Mapping[str, Any] | None, str | None, dict[str, Any]]:
    settings_aliases: Mapping[str, Any] | None = None
    settings_crs: str | None = None

    if isinstance(settings, LoadSettings):
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
                raise TypeError("Project.load settings['aliases'] must be a mapping")
            settings_aliases = raw_aliases
        raw_crs = settings_dict.pop("crs", None)
        if raw_crs is not None:
            settings_crs = str(raw_crs)
    else:
        raise TypeError("Project.load settings must be a mapping or LoadSettings")

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
            raise TypeError("Project.load aliases values must be strings or iterables of strings")
    return out


def _well_attr_lookup(attr: str, well_ids: Iterable[str]) -> str | None:
    for well_id in well_ids:
        if attr == well_id or attr == _identifier_for(well_id):
            return well_id
    return None


def _resolve_collection_name(name: str, names: Iterable[str]) -> str | None:
    names_list = list(names)
    if name in names_list:
        return name
    token = _lookup_token(name)
    matches = [candidate for candidate in names_list if _lookup_token(candidate) == token]
    if len(matches) == 1:
        return matches[0]
    leaf_matches = [
        candidate
        for candidate in names_list
        if _lookup_token(str(candidate).rsplit(".", 1)[-1]) == token
    ]
    if len(leaf_matches) == 1:
        return leaf_matches[0]
    return None


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
            skipped.append(_skip(root, path, SKIP_LOAD_ERROR, str(exc)))
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
            skipped.append(_skip(root, files, SKIP_LOAD_ERROR, f"load_well {well_id}: {exc}"))
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
            skipped.append(_skip(root, rec.path, SKIP_LOAD_ERROR, f"load_well_tops: {exc}"))
            continue
        inventory["tops"].append(name)
        try:
            tables[name] = _parse_petrel_tops_table(rec.path)
        except Exception as exc:
            skipped.append(_skip(root, rec.path, SKIP_LOAD_ERROR, f"parse_well_tops: {exc}"))
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
                geo.load_surface(name, str(rec.path))
                inventory["surfaces"].append(name)
            elif role == "points":
                geo.load_points(name, str(rec.path))
                inventory["points"].append(name)
            elif role == "polygons":
                geo.load_polygons(name, str(rec.path))
                inventory["polygons"].append(name)
        except Exception as exc:
            skipped.append(_skip(root, rec.path, SKIP_LOAD_ERROR, f"load_{role}: {exc}"))


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
    return counts


def _spatial_role(path: Path, kind: FormatKind) -> str | None:
    suffix = path.suffix.lower()
    if kind in (FormatKind.IrapClassicGrid, FormatKind.Cps3Grid):
        return "surface"
    if kind in (FormatKind.EarthVisionGrid, FormatKind.IrapClassicPoints):
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
    stem = rel.with_suffix("").as_posix()
    return stem.replace("/", ".")


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
