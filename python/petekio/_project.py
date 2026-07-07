"""Project-level raw-tree loading facade for petekIO.

`Project` is intentionally a Python facade over `GeoData`: it owns scan metadata
and user-facing inventory, while all loaded subsurface data remains in the Rust
`GeoData` substrate.
"""

from __future__ import annotations

import json
from collections.abc import Iterable, Iterator, Mapping
from dataclasses import dataclass
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


class _NamedCollection:
    """Small mapping-like view over Project names resolved through GeoData."""

    def __init__(self, names: Iterable[str], getter):
        self._names = list(names)
        self._getter = getter

    def __len__(self) -> int:
        return len(self._names)

    def __iter__(self) -> Iterator[Any]:
        return iter(self.values())

    def __contains__(self, name: object) -> bool:
        return name in self._names

    def __getitem__(self, name: str) -> Any:
        value = self._getter(name)
        if value is None:
            raise KeyError(name)
        return value

    def __call__(self, name: str) -> Any:
        return self._getter(name)

    def get(self, name: str, default: Any = None) -> Any:
        value = self._getter(name)
        return default if value is None else value

    def names(self) -> list[str]:
        return list(self._names)

    def values(self) -> list[Any]:
        return [self._getter(name) for name in self._names]

    def items(self) -> list[tuple[str, Any]]:
        return [(name, self._getter(name)) for name in self._names]

    def __repr__(self) -> str:
        return f"NamedCollection({self._names!r})"


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
    ) -> None:
        self._geo = geo
        self.source = None if source is None else str(source)
        self.aliases = dict(aliases or {})
        self.crs = crs
        self.settings = dict(settings or {})
        self._inventory = dict(inventory or self._empty_inventory(self.source))
        self._log_resolution_cache: dict[str, list[dict[str, Any]]] = {}

    @classmethod
    def load(
        cls,
        path: str | Path,
        aliases: Mapping[str, Any] | None = None,
        crs: str | None = None,
        settings: Mapping[str, Any] | None = None,
    ) -> "Project":
        """Load a `.pproj` or recursively ingest a raw Petrel-style directory."""

        src = Path(path)
        settings_dict = dict(settings or {})
        if src.suffix.lower() == ".pproj":
            geo = GeoData.open(str(src))
            return cls(
                geo,
                source=src,
                aliases=aliases,
                crs=crs,
                settings=settings_dict,
                inventory=cls._inventory_from_pproj(src),
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
        _load_petrel_tops(geo, src, records, inventory, skipped)
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
        return self._geo.wells

    @property
    def tops(self) -> list[str]:
        return list(self._inventory.get("tops", []))

    @property
    def logs(self) -> Any:
        """Lazy log-expression namespace for static workflow recipes."""

        from ._logs import Logs

        return Logs(self)

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

        channel = self.logs.validate(_coerce_log_channel(source, self.logs))
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
) -> None:
    for rec in records:
        if rec.kind != FormatKind.PetrelTops:
            continue
        try:
            geo.load_well_tops(str(rec.path))
        except Exception as exc:
            skipped.append(_skip(root, rec.path, SKIP_LOAD_ERROR, f"load_well_tops: {exc}"))
            continue
        inventory["tops"].append(_name_for(root, rec.path))


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

        name = _name_for(root, rec.path)
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


def _skip(root: Path, path: Path, reason: str, detail: str) -> dict[str, str]:
    try:
        display = path.relative_to(root).as_posix()
    except ValueError:
        display = str(path)
    return {"path": display, "reason": reason, "detail": detail}
