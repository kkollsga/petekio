"""petekio._viewer — the standalone WellLogBundle producer (petekio's slice of
the well-correlation seam).

This is petekio's **producer** for the fourth viewer view (well correlation): it
turns a petekio well's own logs + trajectory + tops into a ``WellLogBundle``
(``kind: "wells_logs"``, ``schema_version: 4``) and hands it to the viewer unit.
It is the *viewer-unit second-consumer proof* — the same bundle the reference
fixture (``petektools/viewer/_wells.py``) emits, produced from real petekio data.

**Seam contract.** The wire format is ratified in
``petekSuite/dev-docs/designs/well-log-bundle-seam.md`` and codified as
implemented in ``petektools/viewer/SCHEMA.md`` (the ``WellLogBundle`` section).
Per the petek family **coupling rule** — share conventions, not code, and only
downward through the DAG — the wire schema + the v3 lane encoding are
**DUPLICATED here** rather than imported from petektools: petekio depends on
neither peteksim nor petekstatic, and treats ``petektools.viewer`` as an
**optional runtime dependency** (lazily imported in :meth:`LogSession.serve` /
:meth:`LogSession.save`, with a helpful error if the wheel is absent). This
module is the documented **seam-twin** of that contract.

**Division of labour.** petekio's Rust side gathers the raw per-well data (a
shared ``md``/``tvd`` grid + each curve resampled onto it, canonicalized
mnemonics, units, and zones); this pure-Python module owns the *wire format* —
the JSON header, the base64 f32 lane blocks, per-curve ``range``/``cutoff``/
``codes`` extras, tops ordering validation, and the optional serve/save
delegation. Keeping the wire producer in Python makes the lane encoding directly
testable (bit-exact f32, NaN policy) and the bundle round-trippable against the
reference fixture without a Rust build.
"""

from __future__ import annotations

import base64
import math
import struct
import sys
from array import array
from typing import TYPE_CHECKING, Any, Dict, List, Optional, Sequence, Union

from ._specs import phie_cutoff_value

if TYPE_CHECKING:  # avoid a runtime import cycle; used only for annotations
    from ._specs import ViewSettings, ViewSpec

#: Bundle schema version this producer targets (the ``wells_logs`` kind under v4).
SCHEMA_VERSION = 4

#: Canonical quiet-NaN f32 bit pattern (matches the engine + viewer: 0x7FC00000).
NAN_F32 = struct.unpack("<f", struct.pack("<I", 0x7FC00000))[0]

#: Curves rendered as a categorical **flag strip** (net/facies) rather than a
#: continuous polyline, keyed by canonical mnemonic (upper-case). A caller may
#: extend this per call via ``flags=[...]``. NTG is deliberately continuous.
_FLAG_NAMES = {"FACIES", "LITH", "LITHOLOGY", "NETFLAG"}


def _le_bytes_f32(values: Sequence[Optional[float]]) -> bytes:
    """Pack ``values`` as tightly-packed little-endian ``f32`` bytes.

    ``None`` and non-finite (NaN/inf) entries pack as the canonical quiet-NaN
    ``0x7FC00000`` (the viewer reads NaN as null → the curve breaks). This is the
    exact block encoding the volume decode kernel already consumes, so a log lane
    rides the viewer's existing decode path (no special casing).
    """
    clean = [
        NAN_F32 if (v is None or not math.isfinite(v)) else float(v) for v in values
    ]
    a = array("f", clean)
    if a.itemsize != 4:  # platform sanity — every target we ship matches
        raise RuntimeError(f"array('f').itemsize={a.itemsize} != 4")
    if sys.byteorder == "big":
        a = array("f", a)
        a.byteswap()
    return a.tobytes()


def encode_lane(values: Sequence[Optional[float]]) -> Dict[str, Any]:
    """Encode one f32 lane as a v3-style base64 binary block ``{dtype, shape, data}``.

    The block shape is exactly the viewer's contract:
    ``{"dtype": "f32", "shape": [n], "data": "<base64 little-endian>"}`` with
    ``NaN`` = ``0x7FC00000``. Byte-identical to the reference fixture's
    ``encode_lane`` for the same input — the seam-twin round-trip anchor.
    """
    raw = _le_bytes_f32(values)
    return {
        "dtype": "f32",
        "shape": [len(values)],
        "data": base64.b64encode(raw).decode("ascii"),
    }


def _finite(values: Sequence[float]) -> List[float]:
    return [v for v in values if v is not None and math.isfinite(v)]


def _range(values: Sequence[float]) -> Dict[str, float]:
    """A ``{min, max}`` fixing a continuous track's hi–lo header scale."""
    fin = _finite(values)
    if not fin:
        return {"min": 0.0, "max": 1.0}
    return {"min": min(fin), "max": max(fin)}


def _codes(values: Sequence[float]) -> Dict[str, str]:
    """Flag-strip code → label map. petekio carries no facies dictionary, so the
    label is the integer code itself (``{"0": "0", "1": "1"}``); the viewer draws
    the zero code recessive and each non-zero code in an identity slot."""
    ints = sorted({int(round(v)) for v in _finite(values)})
    return {str(c): str(c) for c in ints}


def _is_flag(canonical: str, mnemonic: str, flags: Optional[Sequence[str]]) -> bool:
    """Whether a curve renders as a flag strip. Auto by name (``FACIES``/``LITH``/
    ``…FLAG``) or explicitly listed in ``flags`` (matched on canonical or raw
    mnemonic, case-insensitive)."""
    if flags is not None:
        wanted = {f.strip().upper() for f in flags}
        if canonical.upper() in wanted or mnemonic.upper() in wanted:
            return True
    u = canonical.upper()
    return u in _FLAG_NAMES or u.endswith("FLAG") or u.endswith("FACIES")


def _build_tops(
    zones_raw: Sequence[Dict[str, Any]],
    tops: Union[None, bool, Sequence[str]],
):
    """Build the ``tops[]`` picks + ``zones[]`` bands from the well's raw zones,
    or ``(None, None)`` when the caller did not ask for tops.

    ``tops`` is opt-in per the seam contract (a standalone logs view carries no
    picks): ``None``/``False`` → omit; ``True`` → all the well's zone tops;
    a list of horizon names → only those (case-insensitive, well order preserved).
    Picks + zone bands are emitted top→down in the well's zone order; the pick
    TVDs are validated **strictly increasing** (loud error on an out-of-order /
    overturned stack). A well simply missing a formation contributes no pick for
    it (missing-pick passthrough) rather than failing.
    """
    if tops is None or tops is False:
        return None, None
    keep = None
    if not isinstance(tops, bool):
        keep = {str(t).strip().lower() for t in tops}

    picks: List[Dict[str, Any]] = []
    zones: List[Dict[str, Any]] = []
    for z in zones_raw:
        name = z["name"]
        if keep is not None and name.strip().lower() not in keep:
            continue
        top_tvd = round(float(z["top_tvd"]), 4)
        base_tvd = round(float(z["base_tvd"]), 4)
        picks.append({"horizon": name, "tvd_m": top_tvd})
        zones.append({"name": name, "top_tvd_m": top_tvd, "base_tvd_m": base_tvd})

    for a, b in zip(picks, picks[1:]):
        if not (b["tvd_m"] > a["tvd_m"]):
            raise ValueError(
                "well tops are not sorted top->down by TVD: "
                f"'{a['horizon']}' @ {a['tvd_m']} m then "
                f"'{b['horizon']}' @ {b['tvd_m']} m (a pick must lie strictly "
                "below the one above it)"
            )
    return picks, zones


def _log_well(
    raw: Dict[str, Any],
    *,
    tops: Union[None, bool, Sequence[str]],
    phie_cutoff: Optional[float],
    flags: Optional[Sequence[str]],
) -> Dict[str, Any]:
    """One ``LogWell`` wire object from one well's raw data (see module docstring
    for the raw shape petekio's Rust side hands in)."""
    md = raw["md"]
    tvd = raw["tvd"]

    curves: List[Dict[str, Any]] = []
    for c in raw["curves"]:
        canonical = c["canonical"]
        mnemonic = c["mnemonic"]
        values = c["values"]
        entry: Dict[str, Any] = {
            "mnemonic": canonical,  # canonical — petekio is the family name authority
            "display_name": mnemonic,  # the raw source mnemonic, for the header
            "unit": c.get("unit", ""),
            "values": encode_lane(values),
        }
        if _is_flag(canonical, mnemonic, flags):
            entry["kind"] = "flag"
            entry["codes"] = _codes(values)
        else:
            entry["kind"] = "continuous"
            entry["range"] = _range(values)
            if canonical.upper() == "PHIE" and phie_cutoff is not None:
                entry["cutoff"] = float(phie_cutoff)
        curves.append(entry)

    picks, zones = _build_tops(raw.get("zones", []), tops)

    well: Dict[str, Any] = {
        "id": raw["id"],
        "display_name": raw.get("display_name", raw["id"]),
        "x": raw["x"],
        "y": raw["y"],
        "datum_m": raw["datum_m"],
        "md_m": encode_lane(md),
        "tvd_m": encode_lane(tvd),
        "curves": curves,
    }
    if picks is not None:
        well["tops"] = picks
        well["zones"] = zones
    # NO `ties` here — tie residuals are model context only (peteksim's slice);
    # a standalone petekio bundle never carries them (seam contract).
    return well


def build_well_log_bundle(
    raws: Sequence[Dict[str, Any]],
    *,
    spec: Optional["ViewSpec"] = None,
    tops: Union[None, bool, Sequence[str]] = None,
    flatten_default: Optional[str] = None,
    phie_cutoff: Optional[float] = 0.08,
    flags: Optional[Sequence[str]] = None,
) -> Dict[str, Any]:
    """Assemble a ``WellLogBundle`` (``kind: "wells_logs"``, ``schema_version``
    4) from petekio raw well data.

    ``raws`` is the list of per-well raw dicts (from petekio's Rust gatherer).
    A :class:`petekio.ViewSpec` (``spec=``) supplies WHAT to show declaratively;
    the legacy ``tops``/``flatten_default``/``phie_cutoff``/``flags`` kwargs are
    the per-call alternative (passing both a ``spec`` and any of them is a loud
    error). ``tops`` opts picks/zones in (see :func:`_build_tops`); when tops are
    included, ``flatten_default`` selects the pre-selected flatten pick (defaults
    to the first well's first pick). ``phie_cutoff`` (default the petekio net
    default 0.08) is drawn as the PHIE net cutoff line + reservoir fill.
    ``flags`` lists extra curves to render as categorical strips.

    See ``petekSuite/dev-docs/designs/well-log-bundle-seam.md`` and
    ``petektools/viewer/SCHEMA.md``.
    """
    if spec is not None:
        legacy = (
            tops is not None
            or flatten_default is not None
            or flags is not None
            or (phie_cutoff is not None and phie_cutoff != 0.08)
        )
        if legacy:
            raise ValueError(
                "build_well_log_bundle: pass EITHER spec=ViewSpec(...) OR the legacy "
                "tops/flatten_default/phie_cutoff/flags kwargs, not both"
            )
        tops = spec.tops
        flatten_default = spec.flatten_default
        flags = spec.flags
        phie_cutoff = phie_cutoff_value(spec.cutoff)

    wells = [
        _log_well(r, tops=tops, phie_cutoff=phie_cutoff, flags=flags)
        for r in raws
        if r.get("curves")  # skip a well with no curves — nothing to correlate
    ]

    fd: Optional[str] = None
    if tops is not None and tops is not False:
        first = next((w for w in wells if w.get("tops")), None)
        if first is not None:
            fd = flatten_default or first["tops"][0]["horizon"]

    return {
        "kind": "wells_logs",
        "schema_version": SCHEMA_VERSION,
        "flatten_default": fd,
        "wells": wells,
    }


class LogSession:
    """A logs-only viewer session over one or more wells — the standalone
    ``well.view()`` surface. Holds the built ``WellLogBundle`` and mirrors the
    viewer unit's ergonomics: :meth:`serve` (non-blocking local server) and
    :meth:`save` (one self-contained HTML file). Both delegate to
    ``petektools.viewer`` (the optional runtime dependency), imported lazily so
    petekio stays usable without it."""

    def __init__(self, payload: Dict[str, Any]) -> None:
        self.payload = payload

    def bundle(self) -> Dict[str, Any]:
        """The raw ``WellLogBundle`` dict (for inspection / round-trip tests)."""
        return self.payload

    @staticmethod
    def _viewer():
        try:
            from petektools import viewer  # optional runtime dependency
        except ImportError as exc:  # pragma: no cover - exercised only when absent
            raise ImportError(
                "well.view() renders through the viewer unit, which is not "
                "installed. Install it with `pip install petektools` "
                "(provides petektools.viewer). petekio produces the "
                "WellLogBundle; petektools.viewer renders it."
            ) from exc
        return viewer

    def serve(self, **kwargs: Any):
        """Serve the bundle on a non-blocking local viewer server (returns the
        server handle / URL from ``petektools.viewer.serve``)."""
        return self._viewer().serve(self.payload, **kwargs)

    def save(self, path: str, **kwargs: Any) -> str:
        """Write one self-contained HTML file (``petektools.viewer.save_view``)
        and return ``path``."""
        self._viewer().save_view(self.payload, path, **kwargs)
        return path


def render(
    data: Union[Dict[str, Any], Sequence[Dict[str, Any]]],
    *,
    spec: Optional["ViewSpec"] = None,
    settings: Optional["ViewSettings"] = None,
    tops: Union[None, bool, Sequence[str]] = None,
    flatten_default: Optional[str] = None,
    phie_cutoff: Optional[float] = 0.08,
    flags: Optional[Sequence[str]] = None,
    serve: bool = True,
    save: Optional[str] = None,
) -> LogSession:
    """Build a :class:`LogSession` from petekio raw well data and (by default)
    serve it. ``data`` is one raw well dict (``well.view()``) or a list of them
    (a multi-well session). This is the entry point petekio's Rust ``view()``
    trampolines call.

    A :class:`petekio.ViewSpec` (``spec=``) declares WHAT to show and a
    :class:`petekio.ViewSettings` (``settings=``) HOW to deliver it; each
    supersedes the matching legacy kwargs (WHAT: tops/flatten_default/
    phie_cutoff/flags; HOW: serve/save). With ``save=<path>`` the bundle is
    written to a self-contained HTML file instead of served; ``serve=False``
    builds the session without opening anything."""
    raws = [data] if isinstance(data, dict) else list(data)
    if settings is not None:
        serve = settings.serve
        save = settings.save
    payload = build_well_log_bundle(
        raws,
        spec=spec,
        tops=tops,
        flatten_default=flatten_default,
        phie_cutoff=phie_cutoff,
        flags=flags,
    )
    session = LogSession(payload)
    if save is not None:
        session.save(save)
    elif serve:
        session.serve()
    return session
