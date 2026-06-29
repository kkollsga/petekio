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

## Built in gates

petekIO grows in **gated phases** against a locked contract: every public
signature is specified in [`API.md`](API.md) (a change needs sign-off), and the
design + build roadmap live in [`SPEC.md`](SPEC.md).

> **Status:** early development. The public API is locked and the core data path
> (ingest → normalize → validate → interpret → characterise) is in place; breadth
> is still filling in — more ingest formats, fluid contacts, richer
> interpretation.

## Design at a glance

- **Strictly layered, one-way deps:** `foundation → io → core → analysis →
  manager → py`.
- **A manager substrate** (`GeoData`): load once, operations broadcast across the
  collection — no per-item loops.
- **Domain objects carry their operations** (arithmetic, filters, interpolation,
  stats) — fluent and chainable.
- **Rust core + thin PyO3**; the Python API mirrors the Rust API.

## Built on

- **petekAlgorithms** — standalone numerics / geostatistics kernels (gridding,
  interpolation) that petekIO builds on.

## License

MIT — see [LICENSE-MIT](LICENSE-MIT).
