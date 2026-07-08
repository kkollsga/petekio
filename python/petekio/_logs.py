"""Lazy well-log expression objects for static property recipes."""

from __future__ import annotations

from collections.abc import Iterable, Mapping
from numbers import Real
from typing import Any

from ._petekio import canonical_mnemonic


_COMPARISONS = {">", ">=", "<", "<=", "==", "!="}
_COMPOSITION = {"and", "or", "not"}
_TESTS = {"is_finite", "is_null", "not_null"}
_ARITHMETIC = {"+", "-", "*", "/"}


class Logs:
    """Project wells log namespace with lazy channel expression access."""

    def __init__(self, project: Any) -> None:
        self._project = project
        self._actual = self._discover_actual_mnemonics(project)
        self._aliases = _normalise_aliases(getattr(project, "aliases", None))
        self._canonical = self._canonical_index(self._actual.values())

    def __len__(self) -> int:
        return len(self.names())

    def __iter__(self):
        return iter(self.names())

    def __getitem__(self, name: int | slice | str) -> "LogChannel | str | list[str]":
        if isinstance(name, (int, slice)):
            return self.names()[name]
        return self.channel(name)

    def __getattr__(self, name: str) -> "LogChannel":
        if name.startswith("_"):
            raise AttributeError(name)
        try:
            return self.channel(name)
        except KeyError as exc:
            raise AttributeError(str(exc)) from None

    def __contains__(self, name: object) -> bool:
        if not isinstance(name, str):
            return False
        try:
            self.resolve(name)
        except KeyError:
            return False
        return True

    def channel(self, name: str) -> "LogChannel":
        requested, resolved = self.resolve(name)
        return LogChannel(self, requested=requested, mnemonic=resolved)

    def resolve(self, name: str) -> tuple[str, str]:
        """Return ``(requested, resolved_mnemonic)`` or raise ``KeyError``."""

        requested = _clean_name(name)
        if not requested:
            raise KeyError("empty log mnemonic")

        key = _lookup_key(requested)
        if key in self._actual:
            return requested, self._actual[key]

        alias = self._aliases.get(key)
        if alias:
            alias_key = _lookup_key(alias)
            if alias_key in self._actual:
                return requested, self._actual[alias_key]
            return requested, alias

        canonical = canonical_mnemonic(requested)
        canonical_key = _lookup_key(canonical)
        matches = self._canonical.get(canonical_key, ())
        if len(matches) == 1:
            return requested, matches[0]
        if len(matches) > 1:
            choices = ", ".join(sorted(matches))
            raise KeyError(f"ambiguous log mnemonic '{requested}' resolves to {choices}")

        available = ", ".join(self.names()) or "none loaded"
        raise KeyError(f"unknown log mnemonic '{requested}' (available: {available})")

    def names(self) -> list[str]:
        """Loaded log mnemonics known to this project."""

        return sorted(set(self._actual.values()), key=str.casefold)

    def aliases(self) -> dict[str, str]:
        """Flattened alias map as ``raw_or_alias -> loaded_canonical``."""

        return dict(self._aliases)

    def validate(self, expr: "LogChannel | LogPredicate") -> "LogChannel | LogPredicate":
        """Validation hook for downstream recipe builders."""

        if isinstance(expr, LogChannel):
            self.resolve(expr.requested)
            if expr.filter is not None:
                self.validate(expr.filter)
            return expr
        if isinstance(expr, LogPredicate):
            for operand in expr.operands:
                if isinstance(operand, (LogChannel, LogPredicate)):
                    self.validate(operand)
            return expr
        raise TypeError(f"expected LogChannel or LogPredicate, got {type(expr).__name__}")

    def to_dict(self) -> dict[str, Any]:
        return {
            "kind": "logs",
            "channels": self.names(),
            "aliases": self.aliases(),
        }

    as_dict = to_dict

    def __eq__(self, other: object) -> bool:
        if isinstance(other, Logs):
            return self.names() == other.names()
        if isinstance(other, list):
            return self.names() == other
        return False

    def __repr__(self) -> str:
        return repr(self.names())

    __str__ = __repr__

    @staticmethod
    def _discover_actual_mnemonics(project: Any) -> dict[str, str]:
        actual: dict[str, str] = {}
        for well_id in getattr(project, "_inventory", {}).get("wells", []):
            well = project.well(well_id)
            if well is None:
                continue
            bore_names = _call_or_empty(well, "bores")
            if not bore_names:
                for mnemonic in _call_or_empty(well, "mnemonics"):
                    actual.setdefault(_lookup_key(mnemonic), str(mnemonic))
                continue
            for bore in bore_names:
                sidetrack = well.sidetrack(bore)
                if sidetrack is None:
                    continue
                for mnemonic in _call_or_empty(sidetrack, "mnemonics"):
                    actual.setdefault(_lookup_key(mnemonic), str(mnemonic))
        return actual

    @staticmethod
    def _canonical_index(mnemonics: Iterable[str]) -> dict[str, tuple[str, ...]]:
        index: dict[str, list[str]] = {}
        for mnemonic in mnemonics:
            key = _lookup_key(canonical_mnemonic(mnemonic))
            index.setdefault(key, []).append(mnemonic)
        return {key: tuple(values) for key, values in index.items()}


class _LogArithmeticMixin:
    def __add__(self, other: Any) -> "LogExpression":
        return LogExpression("+", (self, _coerce_log_operand(other)))

    def __radd__(self, other: Any) -> "LogExpression":
        return LogExpression("+", (_coerce_log_operand(other), self))

    def __sub__(self, other: Any) -> "LogExpression":
        return LogExpression("-", (self, _coerce_log_operand(other)))

    def __rsub__(self, other: Any) -> "LogExpression":
        return LogExpression("-", (_coerce_log_operand(other), self))

    def __mul__(self, other: Any) -> "LogExpression":
        return LogExpression("*", (self, _coerce_log_operand(other)))

    def __rmul__(self, other: Any) -> "LogExpression":
        return LogExpression("*", (_coerce_log_operand(other), self))

    def __truediv__(self, other: Any) -> "LogExpression":
        return LogExpression("/", (self, _coerce_log_operand(other)))

    def __rtruediv__(self, other: Any) -> "LogExpression":
        return LogExpression("/", (_coerce_log_operand(other), self))


class LogChannel(_LogArithmeticMixin):
    """Lazy reference to one log channel, optionally filtered by a predicate."""

    def __init__(
        self,
        logs: Logs,
        *,
        requested: str,
        mnemonic: str,
        filter: "LogPredicate | None" = None,
    ) -> None:
        self._logs = logs
        self.requested = requested
        self.mnemonic = mnemonic
        self.filter = filter

    @property
    def name(self) -> str:
        return self.mnemonic

    def __call__(self, predicate: "LogPredicate") -> "LogChannel":
        return self.where(predicate)

    def where(self, predicate: "LogPredicate") -> "LogChannel":
        if not isinstance(predicate, LogPredicate):
            raise TypeError("LogChannel.where() expects a LogPredicate")
        self._logs.validate(predicate)
        return LogChannel(
            self._logs,
            requested=self.requested,
            mnemonic=self.mnemonic,
            filter=predicate,
        )

    def is_finite(self) -> "LogPredicate":
        return LogPredicate("is_finite", (self,))

    def is_null(self) -> "LogPredicate":
        return LogPredicate("is_null", (self,))

    def not_null(self) -> "LogPredicate":
        return LogPredicate("not_null", (self,))

    def to_basis(
        self,
        basis: "LogChannel | LogBasis",
        *,
        interpolation: str = "linear",
    ) -> "LogBasis":
        return LogBasis(self, basis=basis, interpolation=interpolation)

    def to_dict(self) -> dict[str, Any]:
        data: dict[str, Any] = {
            "kind": "log_channel",
            "mnemonic": self.mnemonic,
            "requested": self.requested,
        }
        if self.filter is not None:
            data["filter"] = self.filter.to_dict()
        return data

    as_dict = to_dict

    def __gt__(self, other: Any) -> "LogPredicate":
        return self._compare(">", other)

    def __ge__(self, other: Any) -> "LogPredicate":
        return self._compare(">=", other)

    def __lt__(self, other: Any) -> "LogPredicate":
        return self._compare("<", other)

    def __le__(self, other: Any) -> "LogPredicate":
        return self._compare("<=", other)

    def __eq__(self, other: Any) -> "LogPredicate":  # type: ignore[override]
        return self._compare("==", other)

    def __ne__(self, other: Any) -> "LogPredicate":  # type: ignore[override]
        return self._compare("!=", other)

    def _compare(self, op: str, other: Any) -> "LogPredicate":
        if op not in _COMPARISONS:
            raise ValueError(f"unsupported log comparison {op!r}")
        return LogPredicate(op, (self, _coerce_operand(other)))

    def __bool__(self) -> bool:
        raise TypeError("LogChannel objects are lazy expressions; compare them to build predicates")

    def __repr__(self) -> str:
        base = f"logs.{self.requested}"
        if self.mnemonic != self.requested:
            base += f"[{self.mnemonic}]"
        if self.filter is not None:
            base += f".where({self.filter!r})"
        return base


class LogExpression(_LogArithmeticMixin):
    """Lazy arithmetic expression over log channels/scalars."""

    def __init__(self, op: str, operands: tuple[Any, Any]) -> None:
        if op not in _ARITHMETIC:
            raise ValueError(f"unsupported log arithmetic operator {op!r}")
        self.op = op
        self.operands = operands

    def to_dict(self) -> dict[str, Any]:
        return {
            "kind": "log_expression",
            "op": self.op,
            "operands": [_operand_to_dict(operand) for operand in self.operands],
        }

    as_dict = to_dict

    def __bool__(self) -> bool:
        raise TypeError("LogExpression objects are lazy expressions; assign or resolve them")

    def __repr__(self) -> str:
        return f"({self.operands[0]!r} {self.op} {self.operands[1]!r})"


class LogBasis(_LogArithmeticMixin):
    """Lazy log operand explicitly resampled to another log/basis."""

    def __init__(
        self,
        source: LogChannel,
        *,
        basis: "LogChannel | LogBasis",
        interpolation: str = "linear",
    ) -> None:
        if not isinstance(source, LogChannel):
            raise TypeError("LogBasis source must be a LogChannel")
        if not isinstance(basis, (LogChannel, LogBasis)):
            raise TypeError("LogBasis basis must be a LogChannel or LogBasis")
        self.source = source
        self.basis = basis
        self.interpolation = _normalise_interpolation(interpolation)

    @property
    def requested(self) -> str:
        return self.source.requested

    @property
    def mnemonic(self) -> str:
        return self.source.mnemonic

    def to_basis(
        self,
        basis: "LogChannel | LogBasis",
        *,
        interpolation: str = "linear",
    ) -> "LogBasis":
        return LogBasis(self.source, basis=basis, interpolation=interpolation)

    def to_dict(self) -> dict[str, Any]:
        return {
            "kind": "log_basis",
            "source": self.source.to_dict(),
            "basis": self.basis.to_dict(),
            "interpolation": self.interpolation,
        }

    as_dict = to_dict

    def __repr__(self) -> str:
        return f"{self.source!r}.to_basis({self.basis!r}, interpolation={self.interpolation!r})"


class LogPredicate:
    """Lazy predicate over one or more log channels."""

    def __init__(self, op: str, operands: tuple[Any, ...]) -> None:
        if op not in _COMPARISONS | _COMPOSITION | _TESTS:
            raise ValueError(f"unsupported log predicate operator {op!r}")
        self.op = op
        self.operands = operands

    def __and__(self, other: Any) -> "LogPredicate":
        return LogPredicate("and", (self, _coerce_predicate(other)))

    def __or__(self, other: Any) -> "LogPredicate":
        return LogPredicate("or", (self, _coerce_predicate(other)))

    def __invert__(self) -> "LogPredicate":
        return LogPredicate("not", (self,))

    def to_dict(self) -> dict[str, Any]:
        return {
            "kind": "log_predicate",
            "op": self.op,
            "operands": [_operand_to_dict(operand) for operand in self.operands],
        }

    as_dict = to_dict

    def __bool__(self) -> bool:
        raise TypeError("LogPredicate objects are lazy expressions; use &, |, and ~ to compose")

    def __repr__(self) -> str:
        if self.op == "not":
            return f"~{self.operands[0]!r}"
        if self.op in {"and", "or"}:
            sep = " & " if self.op == "and" else " | "
            return sep.join(repr(operand) for operand in self.operands)
        if self.op in _TESTS:
            return f"{self.operands[0]!r}.{self.op}()"
        return f"{self.operands[0]!r} {self.op} {self.operands[1]!r}"


def _normalise_aliases(aliases: Mapping[str, Any] | None) -> dict[str, str]:
    if not aliases:
        return {}
    out: dict[str, str] = {}
    for key, value in aliases.items():
        if isinstance(value, str):
            out[_lookup_key(key)] = str(value)
        elif isinstance(value, Iterable):
            canonical = str(key)
            out[_lookup_key(canonical)] = canonical
            for raw in value:
                out[_lookup_key(raw)] = canonical
        else:
            raise TypeError("Project aliases values must be strings or iterables of strings")
    return out


def _call_or_empty(obj: Any, method: str) -> list[Any]:
    fn = getattr(obj, method, None)
    if fn is None:
        return []
    try:
        return list(fn())
    except Exception:
        return []


def _coerce_operand(value: Any) -> Any:
    if isinstance(value, LogChannel):
        return value
    if isinstance(value, bool) or isinstance(value, Real) or value is None:
        return value
    raise TypeError(
        "log comparisons support numeric scalars, None, booleans, and channel-to-channel operands"
    )


def _coerce_log_operand(value: Any) -> Any:
    if isinstance(value, (LogChannel, LogExpression, LogBasis)):
        return value
    if isinstance(value, Real):
        return float(value)
    raise TypeError(
        "log arithmetic supports numeric scalars and log channels/expressions only"
    )


def _coerce_predicate(value: Any) -> "LogPredicate":
    if isinstance(value, LogPredicate):
        return value
    raise TypeError("log predicate composition expects another LogPredicate")


def _operand_to_dict(value: Any) -> dict[str, Any]:
    if isinstance(value, (LogChannel, LogPredicate, LogExpression, LogBasis)):
        return value.to_dict()
    return {"kind": "scalar", "value": value}


def _normalise_interpolation(name: str) -> str:
    n = str(name).strip().casefold().replace("-", "_")
    aliases = {
        "closest": "nearest",
        "nearest": "nearest",
        "linear": "linear",
        "previous": "previous",
        "prev": "previous",
        "ffill": "previous",
        "next": "next",
        "bfill": "next",
        "spline": "spline",
        "cubic": "spline",
    }
    if n not in aliases:
        raise ValueError(
            "unknown log interpolation "
            f"{name!r} (expected nearest, linear, previous, next, or spline)"
        )
    return aliases[n]


def _clean_name(name: str) -> str:
    if not isinstance(name, str):
        raise TypeError(f"log mnemonic must be a string, got {type(name).__name__}")
    return name.strip()


def _lookup_key(name: str) -> str:
    return _clean_name(str(name)).casefold()
