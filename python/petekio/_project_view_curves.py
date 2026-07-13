"""Deterministic metadata-only curve selection for automatic correlations."""

from __future__ import annotations

from collections.abc import Mapping, Sequence
from typing import Any


_FAMILIES = (
    frozenset(("GR", "GAMMA")),
    frozenset(("VSH", "VCL")),
    frozenset(("PHIE", "PHIT", "NPHI")),
    frozenset(("SW", "SWT")),
    frozenset(("PERM", "K")),
    frozenset(("RDEP", "RT", "ILD", "RESD")),
)
_COORDINATES = frozenset(
    (
        "MD",
        "DEPT",
        "DEPTH",
        "TVD",
        "TVDSS",
        "TVDM",
        "X",
        "Y",
        "Z",
        "EASTING",
        "NORTHING",
    )
)
_DISCRETE = frozenset(
    ("FACIES", "LITH", "LITHO", "LITHOLOGY", "NET", "NETFLAG", "CORE", "ZONE")
)


def _key(value: Any) -> str:
    return str(value).strip().upper()


def _fallback_safe(value: str) -> bool:
    key = _key(value)
    if not key or key in _COORDINATES or key in _DISCRETE:
        return False
    return not key.endswith(("FLAG", "FACIES", "CODE", "CLASS"))


def select_auto_curves(
    bore_mnemonics: Mapping[str, Sequence[str]],
    *,
    minimum: int = 4,
    maximum: int = 6,
) -> dict[str, tuple[str, ...]]:
    """Choose a small consistent correlation intent from mnemonic metadata.

    Family order is geological display priority. Within a family each bore
    retains its first exact source mnemonic. If fewer than ``minimum`` family
    intents exist project-wide, safe continuous-looking mnemonic keys are added
    in project/source order and reused consistently wherever present.
    """

    if maximum < 1 or minimum < 0 or minimum > maximum:
        raise ValueError("automatic curve limits require 0 <= minimum <= maximum")
    rows = [(item_id, tuple(map(str, names))) for item_id, names in bore_mnemonics.items()]
    present = {_key(name) for _, names in rows for name in names}
    families = [family for family in _FAMILIES if family & present][:maximum]

    fallback_keys: list[str] = []
    wanted = min(minimum, maximum)
    if len(families) < wanted:
        family_names = set().union(*_FAMILIES)
        for _, names in rows:
            for name in names:
                key = _key(name)
                if (
                    key in family_names
                    or key in fallback_keys
                    or not _fallback_safe(name)
                ):
                    continue
                fallback_keys.append(key)
                if len(families) + len(fallback_keys) >= wanted:
                    break
            if len(families) + len(fallback_keys) >= wanted:
                break

    out: dict[str, tuple[str, ...]] = {}
    for item_id, names in rows:
        chosen: list[str] = []
        for family in families:
            match = next((name for name in names if _key(name) in family), None)
            if match is not None:
                chosen.append(match)
        for fallback in fallback_keys:
            match = next((name for name in names if _key(name) == fallback), None)
            if match is not None:
                chosen.append(match)
        out[item_id] = tuple(chosen[:maximum])
    return out


def template_curve_names(template: Any) -> tuple[str, ...] | None:
    """Extract ordered unique layer mnemonics from a template when possible."""

    if isinstance(template, Mapping):
        data = template
    else:
        to_dict = getattr(template, "to_dict", None)
        if not callable(to_dict):
            return None
        data = to_dict()
        if not isinstance(data, Mapping):
            return None
    tracks = data.get("tracks")
    if not isinstance(tracks, Sequence) or isinstance(tracks, (str, bytes)):
        return None
    names: list[str] = []
    seen: set[str] = set()
    for track in tracks:
        if not isinstance(track, Mapping):
            continue
        layers = track.get("layers", ())
        if not isinstance(layers, Sequence) or isinstance(layers, (str, bytes)):
            continue
        for layer in layers:
            if not isinstance(layer, Mapping):
                continue
            mnemonic = layer.get("mnemonic")
            if not isinstance(mnemonic, str) or not mnemonic.strip():
                continue
            key = _key(mnemonic)
            if key not in seen:
                seen.add(key)
                names.append(mnemonic)
    return tuple(names) if names else None


__all__ = ["select_auto_curves", "template_curve_names"]
