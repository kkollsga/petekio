"""petekio._specs — the pure-Python view spec value-objects.

The petek house **spec pattern** applied to the log-viewer surface:
:class:`ViewSpec` says WHAT to render (curves / tops / flatten / flags / cutoff)
and :class:`ViewSettings` says HOW to deliver it (serve / save). They live in
pure Python — alongside the pure-Python bundle producer :mod:`petekio._viewer`
— so the whole viewer path stays testable without a Rust build, while the two
Rust `view()` trampolines forward them straight through.

Each is a frozen value with the family affordances: ``to_dict``/``from_dict``
(a ``"spec"``-tagged JSON-able dict — a scenario is a savable file), value
equality, ``.replace(...)`` derivation (a new value; the original is
unchanged), and a domain-table ``__repr__`` naming every field.

The φ/Sw/Vsh reservoir cutoffs (``NetSettings``) and the load-time
``IngestSpec`` live on the Rust side (they wrap core `petekio` value types); a
:class:`ViewSpec` ``cutoff`` may be a ``NetSettings`` *or* a bare float.
"""

from __future__ import annotations

from typing import Any, Dict, List, Optional, Sequence, Union

#: Default PHIE net cutoff line (petekio's net default φ ≥ 0.08).
DEFAULT_PHIE_CUTOFF = 0.08


def _cutoff_to_jsonable(cutoff: Any) -> Any:
    """A ViewSpec cutoff (``NetSettings`` | float | None) → a JSON-able value."""
    if cutoff is None:
        return None
    to_dict = getattr(cutoff, "to_dict", None)
    if callable(to_dict):  # a NetSettings
        return to_dict()
    return float(cutoff)


def _cutoff_from_jsonable(value: Any) -> Any:
    """Inverse of :func:`_cutoff_to_jsonable` (rebuilds a ``NetSettings`` from a
    tagged dict, lazily importing the Rust class only when needed)."""
    if value is None or isinstance(value, (int, float)):
        return value
    if isinstance(value, dict) and value.get("spec") == "NetSettings":
        from ._petekio import NetSettings  # lazy: only when a NetSettings is present

        return NetSettings.from_dict(value)
    raise ValueError(f"ViewSpec.cutoff: cannot rebuild from {value!r}")


def phie_cutoff_value(cutoff: Any) -> Optional[float]:
    """The scalar PHIE cutoff line a ``NetSettings`` | float | None yields
    (``NetSettings`` → its ``phi_min``; float → itself; None → None)."""
    if cutoff is None:
        return None
    phi_min = getattr(cutoff, "phi_min", None)
    if phi_min is not None:  # a NetSettings
        return float(phi_min)
    return float(cutoff)


class ViewSpec:
    """WHAT a log-viewer session shows: ``curves`` (mnemonics; None = all),
    ``tops`` (None/False = none, True = all, or a list of horizon names),
    ``flatten_default`` (the pre-selected flatten pick), ``flags`` (extra
    categorical strips), and ``cutoff`` (a ``NetSettings`` supplying the PHIE
    net cutoff line, or a bare float, default 0.08)."""

    __slots__ = ("curves", "tops", "flatten_default", "flags", "cutoff")

    def __init__(
        self,
        curves: Optional[Sequence[str]] = None,
        tops: Union[None, bool, Sequence[str]] = None,
        flatten_default: Optional[str] = None,
        flags: Optional[Sequence[str]] = None,
        cutoff: Any = DEFAULT_PHIE_CUTOFF,
    ) -> None:
        self.curves = list(curves) if curves is not None else None
        self.tops = list(tops) if isinstance(tops, (list, tuple)) else tops
        self.flatten_default = flatten_default
        self.flags = list(flags) if flags is not None else None
        self.cutoff = cutoff

    def to_dict(self) -> Dict[str, Any]:
        """A ``"spec"``-tagged, JSON-able dict (round-trips via :meth:`from_dict`)."""
        return {
            "spec": "ViewSpec",
            "curves": list(self.curves) if self.curves is not None else None,
            "tops": list(self.tops) if isinstance(self.tops, list) else self.tops,
            "flatten_default": self.flatten_default,
            "flags": list(self.flags) if self.flags is not None else None,
            "cutoff": _cutoff_to_jsonable(self.cutoff),
        }

    @classmethod
    def from_dict(cls, d: Dict[str, Any]) -> "ViewSpec":
        """Rebuild from a dict as :meth:`to_dict` emits."""
        return cls(
            curves=d.get("curves"),
            tops=d.get("tops"),
            flatten_default=d.get("flatten_default"),
            flags=d.get("flags"),
            cutoff=_cutoff_from_jsonable(d.get("cutoff", DEFAULT_PHIE_CUTOFF)),
        )

    def replace(self, **changes: Any) -> "ViewSpec":
        """A NEW ViewSpec with the named fields overridden; this one is unchanged."""
        fields = {
            "curves": self.curves,
            "tops": self.tops,
            "flatten_default": self.flatten_default,
            "flags": self.flags,
            "cutoff": self.cutoff,
        }
        unknown = set(changes) - set(fields)
        if unknown:
            raise TypeError(f"ViewSpec.replace: unknown field(s) {sorted(unknown)}")
        fields.update(changes)
        return ViewSpec(**fields)

    def _key(self) -> tuple:
        cutoff = _cutoff_to_jsonable(self.cutoff)
        cutoff_key = tuple(sorted(cutoff.items())) if isinstance(cutoff, dict) else cutoff
        tops = tuple(self.tops) if isinstance(self.tops, list) else self.tops
        return (
            tuple(self.curves) if self.curves is not None else None,
            tops,
            self.flatten_default,
            tuple(self.flags) if self.flags is not None else None,
            cutoff_key,
        )

    def __eq__(self, other: object) -> bool:
        return isinstance(other, ViewSpec) and self._key() == other._key()

    def __hash__(self) -> int:
        return hash(self._key())

    def __repr__(self) -> str:
        return (
            "ViewSpec\n"
            f"  curves:           {self.curves if self.curves is not None else '(all)'}\n"
            f"  tops:             {self.tops}\n"
            f"  flatten_default:  {self.flatten_default}\n"
            f"  flags:            {self.flags}\n"
            f"  cutoff:           {self.cutoff}"
        )


class ViewSettings:
    """HOW a log-viewer session is delivered: ``serve`` (default True — a
    non-blocking local server) and ``save`` (a path to write one self-contained
    HTML file instead)."""

    __slots__ = ("serve", "save")

    def __init__(self, serve: bool = True, save: Optional[str] = None) -> None:
        self.serve = bool(serve)
        self.save = save

    def to_dict(self) -> Dict[str, Any]:
        """A ``"spec"``-tagged, JSON-able dict (round-trips via :meth:`from_dict`)."""
        return {"spec": "ViewSettings", "serve": self.serve, "save": self.save}

    @classmethod
    def from_dict(cls, d: Dict[str, Any]) -> "ViewSettings":
        return cls(serve=d.get("serve", True), save=d.get("save"))

    def replace(self, **changes: Any) -> "ViewSettings":
        """A NEW ViewSettings with the named fields overridden; this one unchanged."""
        fields = {"serve": self.serve, "save": self.save}
        unknown = set(changes) - set(fields)
        if unknown:
            raise TypeError(f"ViewSettings.replace: unknown field(s) {sorted(unknown)}")
        fields.update(changes)
        return ViewSettings(**fields)

    def _key(self) -> tuple:
        return (self.serve, self.save)

    def __eq__(self, other: object) -> bool:
        return isinstance(other, ViewSettings) and self._key() == other._key()

    def __hash__(self) -> int:
        return hash(self._key())

    def __repr__(self) -> str:
        return f"ViewSettings\n  serve:  {self.serve}\n  save:   {self.save}"


__all__: List[str] = ["ViewSpec", "ViewSettings", "phie_cutoff_value", "DEFAULT_PHIE_CUTOFF"]
