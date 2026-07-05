"""The petekIO **spec conformance battery** — the family reference (testing
doctrine R7).

One parametrized module iterating a self-registering registry of every petekio
spec/settings value-object (``NetSettings``, ``IngestSpec``, ``ViewSpec``,
``ViewSettings``). It locks the house spec pattern so the API decisions are
pinned by tests, not prose. Other libraries COPY this module's shape (a family
convention — no shared code sideways); keep it clean and documented.

The battery covers R7's seven rules:

1. **Round-trip** — ``from_dict(to_dict(s)) == s``; ``to_dict`` is a plain,
   JSON-durable dict carrying a ``"spec"`` type tag.
2. **Value semantics** — equal specs compare equal (+ hash where hashable);
   ``.replace(field=…)`` returns a NEW value, original unchanged, result unequal.
3. **Table repr** — ``repr(s)`` names every field (snapshot-pinned per case).
4. **Names-not-objects** — constructing a spec touches no project; apply-time
   resolution of a missing name errors loudly naming BOTH the missing project
   object AND the spec entry (message pinned).
5. **Spec-XOR-kwargs** — a call given both a spec and its legacy kwargs errors
   loudly; legacy-only paths emit ``DeprecationWarning``.
6. **Settings precedence** — a per-call override beats the spec's base value.
7. **Apply determinism** — same spec + same project → identical result (bit).

The **completeness check** (``test_registry_covers_every_spec_type``) fails if a
new petekio spec type ships without a battery entry.
"""

from __future__ import annotations

import json
import os
import warnings
from typing import Any, Callable, Dict, List, Optional

import pytest

import petekio as p


# --------------------------------------------------------------------------- #
# the registry — each spec type self-registers one case                        #
# --------------------------------------------------------------------------- #
class SpecCase:
    """One registry entry: how to build, mutate, and pin a spec type."""

    def __init__(
        self,
        name: str,
        cls: type,
        make: Callable[[], Any],
        make_other: Callable[[], Any],
        replace_kwargs: Dict[str, Any],
        repr_fields: List[str],
    ) -> None:
        self.name = name
        self.cls = cls
        self.make = make
        self.make_other = make_other
        self.replace_kwargs = replace_kwargs
        self.repr_fields = repr_fields


REGISTRY: Dict[str, SpecCase] = {}


def register(case: SpecCase) -> SpecCase:
    REGISTRY[case.name] = case
    return case


register(
    SpecCase(
        name="NetSettings",
        cls=p.NetSettings,
        make=lambda: p.NetSettings(phi_min=0.08, sw_max=0.5, vsh_max=0.4),
        make_other=lambda: p.NetSettings(phi_min=0.12, sw_max=0.5, vsh_max=0.4),
        replace_kwargs={"phi_min": 0.15},
        repr_fields=["phi_min", "sw_max", "vsh_max"],
    )
)
register(
    SpecCase(
        name="IngestSpec",
        cls=p.IngestSpec,
        make=lambda: p.IngestSpec(
            aliases={"PHIE_2025": "PHIE"}, strat_hints=[("A", "B")], unit="m"
        ),
        make_other=lambda: p.IngestSpec(
            aliases={"PHIE_2025": "PHIE"}, strat_hints=[("A", "B")], unit="ft"
        ),
        replace_kwargs={"unit": "ft"},
        repr_fields=["aliases", "hints", "unit"],
    )
)
register(
    SpecCase(
        name="ViewSpec",
        cls=p.ViewSpec,
        make=lambda: p.ViewSpec(curves=["PHIE"], cutoff=0.08),
        make_other=lambda: p.ViewSpec(curves=["SW"], cutoff=0.08),
        replace_kwargs={"flatten_default": "Top A"},
        repr_fields=["curves", "tops", "flatten_default", "flags", "cutoff"],
    )
)
register(
    SpecCase(
        name="ViewSettings",
        cls=p.ViewSettings,
        make=lambda: p.ViewSettings(serve=True, save=None),
        make_other=lambda: p.ViewSettings(serve=False, save=None),
        replace_kwargs={"save": "out.html"},
        repr_fields=["serve", "save"],
    )
)

CASES = list(REGISTRY.values())
_IDS = [c.name for c in CASES]


# --------------------------------------------------------------------------- #
# completeness — a new spec type without a battery entry fails                  #
# --------------------------------------------------------------------------- #
def _discover_spec_types() -> Dict[str, type]:
    """Every public petekio class carrying the spec affordances (to_dict +
    from_dict). The battery must cover exactly this set."""
    found = {}
    for name in petekio_public_names():
        obj = getattr(p, name)
        if isinstance(obj, type) and hasattr(obj, "to_dict") and hasattr(obj, "from_dict"):
            found[name] = obj
    return found


def petekio_public_names() -> List[str]:
    return [n for n in getattr(p, "__all__", dir(p)) if not n.startswith("_")]


def test_registry_covers_every_spec_type():
    discovered = set(_discover_spec_types())
    covered = set(REGISTRY)
    missing = discovered - covered
    assert not missing, (
        f"spec type(s) {sorted(missing)} ship without a conformance-battery entry — "
        "add a SpecCase to REGISTRY (R7 completeness)"
    )
    # And no phantom entries for types that no longer exist.
    assert covered - discovered == set(), f"stale registry entries: {sorted(covered - discovered)}"


# --------------------------------------------------------------------------- #
# R7.1 — dict round-trip with a "spec" type tag, JSON-durable                   #
# --------------------------------------------------------------------------- #
@pytest.mark.parametrize("case", CASES, ids=_IDS)
def test_to_dict_carries_spec_tag(case: SpecCase):
    d = case.make().to_dict()
    assert isinstance(d, dict)
    assert d.get("spec") == case.name, f"{case.name}.to_dict must carry spec='{case.name}'"


@pytest.mark.parametrize("case", CASES, ids=_IDS)
def test_dict_round_trip(case: SpecCase):
    s = case.make()
    assert case.cls.from_dict(s.to_dict()) == s


@pytest.mark.parametrize("case", CASES, ids=_IDS)
def test_dict_is_json_durable(case: SpecCase):
    s = case.make()
    # Survives a full JSON encode/decode (a scenario is a savable file).
    revived = case.cls.from_dict(json.loads(json.dumps(s.to_dict())))
    assert revived == s


# --------------------------------------------------------------------------- #
# R7.2 — value equality + .replace immutability/derivation                     #
# --------------------------------------------------------------------------- #
@pytest.mark.parametrize("case", CASES, ids=_IDS)
def test_value_equality(case: SpecCase):
    assert case.make() == case.make()
    assert case.make() != case.make_other()
    # hash where the type is hashable (equal → equal hash).
    if type(case.make()).__hash__ is not None:
        assert hash(case.make()) == hash(case.make())


@pytest.mark.parametrize("case", CASES, ids=_IDS)
def test_replace_is_immutable_derivation(case: SpecCase):
    base = case.make()
    before = base.to_dict()
    derived = base.replace(**case.replace_kwargs)
    assert derived is not base
    assert derived != base, ".replace must return an unequal derived value"
    assert base.to_dict() == before, ".replace must not mutate the original"


# --------------------------------------------------------------------------- #
# R7.3 — snapshot-pinned table repr naming every field                         #
# --------------------------------------------------------------------------- #
@pytest.mark.parametrize("case", CASES, ids=_IDS)
def test_repr_names_every_field(case: SpecCase):
    r = repr(case.make())
    assert r.startswith(case.name), f"repr should lead with the spec name '{case.name}'"
    for field in case.repr_fields:
        assert field in r, f"{case.name} repr must name field '{field}'"


def test_repr_snapshots_pinned():
    # Exact repr snapshots — the domain tables are a pinned contract.
    assert repr(p.NetSettings(phi_min=0.08, sw_max=0.5, vsh_max=0.4)) == (
        "NetSettings\n  phi_min  >=  0.080\n  sw_max   <=  0.500\n  vsh_max  <=  0.400"
    )
    assert repr(p.ViewSettings(serve=False, save="out.html")) == (
        "ViewSettings\n  serve:  False\n  save:   out.html"
    )


# --------------------------------------------------------------------------- #
# fixtures for the application-level rules (R7.4-7)                             #
# --------------------------------------------------------------------------- #
def _well_dir(root: str, fid: str, phi_scale: float = 1.0, tops: str = "Upper Sand,2000\nBase,2004\n") -> str:
    """A synthetic vertical well: a 5-sample LAS (PHIE/SW) + a formation-tops CSV.
    ``phi_scale`` scales PHIE so two wells differ deterministically."""
    os.makedirs(root, exist_ok=True)
    rows = [(2000, 0.20, 0.30), (2001, 0.05, 0.80), (2002, 0.22, 0.28), (2003, 0.25, 0.20), (2004, 0.18, 0.35)]
    las = (
        "~Version\n VERS. 2.0 :\n WRAP. NO :\n"
        "~Well\n STRT.M 2000.0 :\n STOP.M 2004.0 :\n STEP.M 1.0 :\n NULL. -999.25 :\n"
        "~Curve\n DEPTH.M :\n PHIE.v/v :\n SW.v/v :\n"
        "~ASCII\n"
    )
    for md, phi, sw in rows:
        las += f"{md}.0 {phi * phi_scale:.4f} {sw:.2f}\n"
    with open(os.path.join(root, f"{fid}.las"), "w") as fh:
        fh.write(las)
    with open(os.path.join(root, f"{fid}.csv"), "w") as fh:
        fh.write("name,md\n" + tops)
    return root


def _two_well_project(tmp_path) -> "p.GeoData":
    geo = p.GeoData(unit="m")
    geo.load_well("99/1-1", head=(0.0, 0.0), kb=25.0, files=_well_dir(str(tmp_path / "w1"), "99_1-1", 1.0))
    geo.load_well("99/1-2", head=(0.0, 0.0), kb=25.0, files=_well_dir(str(tmp_path / "w2"), "99_1-2", 1.2))
    return geo


# --------------------------------------------------------------------------- #
# R7.4 — names-not-objects: construction is project-free; apply is loud        #
# --------------------------------------------------------------------------- #
def test_construction_touches_no_project():
    # A spec built against names that resolve to no project object constructs fine.
    p.IngestSpec(strat_hints=[("Nonexistent Fm", "Also Missing")])
    p.ViewSpec(curves=["NOPE"], tops=["Ghost Horizon"])
    p.NetSettings(phi_min=0.99)  # a physically silly but valid value-object


def test_apply_time_missing_name_is_loud(tmp_path):
    geo = _two_well_project(tmp_path)
    with pytest.raises(ValueError) as excinfo:
        geo.load_well_tops(
            str(tmp_path / "w1" / "99_1-1.csv"),
            ingest=p.IngestSpec(strat_hints=[("ZZZ Missing", "Upper Sand")]),
        )
    msg = str(excinfo.value)
    # Names BOTH the spec entry (the hint token) AND the missing project object (a top).
    assert "ZZZ Missing" in msg
    assert "top" in msg and "strat hint" in msg


# --------------------------------------------------------------------------- #
# R7.5 — spec XOR kwargs (loud) + DeprecationWarning on legacy-only            #
# --------------------------------------------------------------------------- #
def test_view_spec_xor_kwargs_is_loud(tmp_path):
    geo = _two_well_project(tmp_path)
    w = geo.well("99/1-1")
    with pytest.raises(ValueError, match="EITHER spec"):
        w.view(spec=p.ViewSpec(curves=["PHIE"]), curves=["PHIE"])
    with pytest.raises(ValueError, match="EITHER settings"):
        w.view(settings=p.ViewSettings(serve=False), serve=False)


def test_load_well_ingest_xor_aliases_is_loud(tmp_path):
    geo = p.GeoData(unit="m")
    with pytest.raises(ValueError, match="EITHER ingest"):
        geo.load_well(
            "99/1-1",
            head=(0.0, 0.0),
            kb=25.0,
            files=_well_dir(str(tmp_path / "w1"), "99_1-1"),
            ingest=p.IngestSpec(unit="m"),
            aliases={"PHIE_2025": "PHIE"},
        )


def test_legacy_paths_emit_deprecation_warning(tmp_path):
    geo = p.GeoData(unit="m")
    with warnings.catch_warnings(record=True) as caught:
        warnings.simplefilter("always")
        geo.load_well(
            "99/1-1",
            head=(0.0, 0.0),
            kb=25.0,
            files=_well_dir(str(tmp_path / "w1"), "99_1-1"),
            aliases={"PHIE_2025": "PHIE"},
        )
        geo.strat_hint("Upper Sand < Base")
    cats = [w.category for w in caught]
    assert sum(issubclass(c, DeprecationWarning) for c in cats) >= 2


# --------------------------------------------------------------------------- #
# R7.6 — settings/override precedence                                          #
# --------------------------------------------------------------------------- #
def test_per_call_override_beats_spec_base(tmp_path):
    geo = _two_well_project(tmp_path)
    bore = geo.well("99/1-1").sidetrack("")
    base = p.NetSettings(phi_min=0.08)
    # cut= supplies the base; the per-call phi_min override wins.
    with_base = dict(bore.net_zone_stats("PHIE", cut=base))
    with_override = dict(bore.net_zone_stats("PHIE", cut=base, phi_min=0.23))
    # phi_min=0.23 keeps only the phi=0.25 sample; base (0.08) keeps 0.20/0.22/0.25/0.18.
    assert with_override["Upper Sand"].mean == pytest.approx(0.25)
    assert with_base["Upper Sand"].mean != pytest.approx(0.25)


# --------------------------------------------------------------------------- #
# R7.7 — apply determinism + the scenario sweep (base vs derived spec)         #
# --------------------------------------------------------------------------- #
def _sweep(geo, cut) -> Dict[str, float]:
    """Net mean PHIE per well under one NetSettings — the swept result."""
    out = {}
    for w in geo.wells.iter():
        bore = w.sidetrack("")
        stats = dict(bore.net_zone_stats("PHIE", cut=cut))
        out[w.id] = stats["Upper Sand"].mean
    return out


def test_scenario_sweep_is_deterministic_and_differs(tmp_path):
    geo = _two_well_project(tmp_path)
    base = p.NetSettings(phi_min=0.08)
    high = base.replace(phi_min=0.25)  # a derived scenario spec (moves BOTH wells)

    base_a = _sweep(geo, base)
    base_b = _sweep(geo, base)
    high_a = _sweep(geo, high)

    # Determinism: same spec + same project → identical (bit) result.
    assert base_a == base_b
    # The derived spec changes the outcome deterministically for every well.
    assert set(base_a) == set(high_a)
    for wid in base_a:
        assert high_a[wid] != base_a[wid], f"scenario sweep did not move well {wid}"
    # And the two wells differ from each other (the fixture's phi_scale).
    ids = list(base_a)
    assert base_a[ids[0]] != base_a[ids[1]]
