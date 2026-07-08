# petekIO

The subsurface **data layer** — a Rust library (with optional Python bindings)
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
does that work once, behind a stable API, so the application on top stays thin
and in its own domain.

- **The whole path, not just parsing.** Files in; normalized, validated,
  interpreted domain objects out.
- **A substrate, not a grab-bag.** Load a project once into a `GeoData`;
  operations broadcast across the whole collection. Immutable, strictly layered,
  fluent, with `history()` on generated domain objects.
- **Rust core, thin Python.** Fast and embeddable, with PyO3 bindings that mirror
  the Rust API.

## Where to start

- **[Install](install.md)** — add the crate or install the wheel.
- **[GeoData project](guide/geodata.md)** — the load-once substrate.
- **[Wells](guide/wells.md)** — bores, trajectories, logs, tops, per-zone stats,
  and the field-wide lithostratigraphic ordering.
- **[Surfaces, points & polygons](guide/surfaces.md)** — gridded surfaces and
  scattered/boundary geometry.
- **[API reference](api/reference.md)** — the public surface, mirroring the
  locked [`API.md`](https://github.com/kkollsga/petekio/blob/main/API.md)
  contract.

!!! note "Status"
    Early development. The public API is locked and the core data path is in
    place; surfaces, multi-bore wells, per-zone stats, and lithostratigraphic
    ordering are landed. Breadth is still filling in.
