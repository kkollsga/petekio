# petekIO

The subsurface **data layer** — a Rust library (with optional PyO3 bindings)
that turns raw subsurface files into clean, validated, interpreted data:
**surfaces, wells (trajectories / tops / logs), points, and polygons**, with
loading, mnemonic and unit normalisation, validation, petrophysical
interpretation, interpolation, and statistics.

The pipeline is the point:

**ingest → normalize → validate → interpret → characterise**

## Why build on it

Subsurface data is the unglamorous, error-prone groundwork under every reservoir
application: vendor LAS mnemonics, mismatched units, out-of-range samples,
cutoffs, gridding that has to honour its control points, uncertainty. petekIO
does that work once and behind a stable API, so the application on top stays thin
and stays in its own domain:

- **The whole path, not just parsing.** Files in; normalized, validated,
  interpreted domain objects out — no re-implementing LAS aliasing, unit
  harmonisation, petrophysical cutoffs (net pay included), or surface
  gridding/resampling further up the stack.
- **Values know what they are.** Results come back in canonical units, each
  carrying an uncertainty distribution and a provenance flag (measured /
  interpolated / defaulted) — so downstream code *propagates* uncertainty rather
  than re-deriving it.
- **A substrate, not a grab-bag.** Load a project once into a `GeoData` and
  operations broadcast across the whole collection. Immutable, strictly layered,
  fluent.
- **Rust core, thin Python.** Fast and embeddable, with PyO3 bindings that mirror
  the Rust API.

## Install

Rust:

```toml
[dependencies]
petekio = "0.2"
```

Python (PyO3 wheel):

```bash
pip install petekio
```

## Quickstart (Python)

Load a project once, then read interpreted results — no parsing or interpolation
in your own code:

```python
import petekio

geo = petekio.GeoData(unit="m")

# A surface (IRAP classic) — sample, stats, volumetrics, resample.
top = geo.load_surface("top_res", "surfaces/top_res.irap")
top.stats.mean
top.area_below(2400)

# A multi-bore well: a Petrel export tree (one bore per .wellpath) + logs.
# head/kb are optional — the .wellpath header fills them.
geo.load_well("15/9-A1", files="wells/15_9-A1/")
geo.load_well_tops("WellTops.tops")        # Horizon picks → matching well + bore

w = geo.well("15/9-A1")
w.bores()                                  # e.g. ["", "A", "B", "ST2"]
bore = w.sidetrack("A")
bore.log_stats("PHIE").mean                # whole-bore curve stats

# Per-zone stats, returned in lithostratigraphic order:
bore.zone_stats("PHIE")                    # [(zone, Stats), ...]
bore.zone_stats("PHIE", "Top A").mean      # one zone directly (None if absent)
geo.strat_order                            # the field's lithostratigraphic column

# A tidy per-zone×bore table (pandas; pip install petekio[pandas]):
w.zone_table("PHIE", stats=("mean", "p50"))  # DataFrame, zone in lithostrat order
```

### Lithostratigraphic ordering

Zones come back in true stratigraphic order, not just measured-depth order.
`load_well_tops` reads **every** well in the tops file and merges their relative
orderings into one field-wide column — so a marker that pinches out (zero
thickness) in one well is ordered correctly by a well that develops it.
Geometry is untouched; only the *order* zones are presented in follows the
column.

## Capabilities

| Domain | What you get |
| --- | --- |
| **Surfaces** | IRAP-classic load, sample/resample (bilinear), arithmetic, stats, `area_below` volumetrics, gridding from scattered points (minimum-curvature) |
| **Wells** | Positioned `.wellpath` trajectories (MD preserved; minimum-curvature interpolation), multi-bore (sidetracks), LAS logs with mnemonic aliasing, Petrel well-tops, per-zone stats, field-wide lithostratigraphic ordering |
| **Points / polygons** | IRAP / GeoJSON / CSV load, clip, point-to-surface gridding |
| **Project** | `GeoData` substrate — load once, broadcast across the collection; views are read-only filtered subsets |

## Built in gates

petekIO grows in **gated phases** against a locked contract: every public
signature is specified in [`API.md`](API.md) (a change needs sign-off), and the
design + build roadmap live in [`SPEC.md`](SPEC.md).

> **Status:** early development. The public API is locked and the core data path
> (ingest → normalize → validate → interpret → characterise) is in place;
> surfaces, multi-bore wells (trajectories / tops / logs), per-zone stats, and
> lithostratigraphic ordering are landed. Breadth is still filling in — more
> ingest formats, fluid contacts, richer interpretation.

## Documentation

- **[API.md](API.md)** — the locked public API contract (Rust, mirrored in Python).
- **[SPEC.md](SPEC.md)** — design constitution + architecture.
- Guides + API reference: the `docs/` site (MkDocs Material; published on Read
  the Docs).

## Design at a glance

- **Strictly layered, one-way deps:** `foundation → algorithms → io → core →
  analysis → manager → py`.
- **A manager substrate** (`GeoData`): load once, operations broadcast across the
  collection — no per-item loops.
- **Domain objects carry their operations** (arithmetic, filters, interpolation,
  stats) — fluent and chainable; immutable (ops return new objects).
- **Algorithms are isolated, QC-able kernels** grouped by discipline (e.g. the
  minimum-curvature survey, the cross-well stratigraphic merge) — pure and
  type-light.
- **Rust core + thin PyO3**; the Python API mirrors the Rust API.

## Built on

- **petekAlgorithms** — standalone numerics / geostatistics kernels (gridding,
  interpolation) that petekIO builds on.

## License

MIT — see [LICENSE-MIT](LICENSE-MIT).
