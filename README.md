# petekIO

The subsurface **data ingestion + structure layer** — a Rust library (with
optional PyO3 bindings) that is the complete input-data model for subsurface
work: **surfaces, wells (trajectories/tops/logs), points, polygons** — with
loading, calculations, interpolation, filters, and statistics built in.

It fills a real gap (no Rust crate does this; xtgeo/welly are Python-only) and
is the data foundation that apps consume so they do **zero** parsing or
interpolation themselves.

> **Status:** early development. The public API is specified and locked in
> [`API.md`](API.md); the design and build roadmap are in [`SPEC.md`](SPEC.md).

## Design at a glance

- **Strictly layered, one-way deps:** `foundation → io → core → analysis →
  manager → py`.
- **A manager substrate** (`GeoData`): load once, operations broadcast across the
  collection — no per-item loops.
- **Domain objects carry their operations** (arithmetic, filters, interpolation,
  stats) — fluent and chainable.
- **Rust core + thin PyO3**; the Python API mirrors the Rust API.

## Part of the petek family

- **petekIO** — this library: the subsurface input-data model + IO.
- **petekSim** — the reservoir simulator that consumes a petekIO `GeoData`.

## License

MIT — see [LICENSE-MIT](LICENSE-MIT).
