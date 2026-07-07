# petekIO guide

petekIO is the subsurface **data layer**: a Rust library (with thin PyO3 bindings)
that turns raw subsurface files into clean, validated, interpreted domain objects
— **surfaces, wells (trajectories / tops / logs), points, and polygons**. The
application on top stays thin because petekIO does the unglamorous groundwork once,
behind a stable API.

The pipeline is the point:

**ingest → normalize → validate → interpret → characterise**

This guide walks the Python surface end to end. Two runnable notebooks accompany
it under `examples/notebooks/`: `01_ingest_tour.ipynb` (author a small project,
load it, inspect it, persist it) and `02_well_analysis.ipynb` (net-cutoff sweeps,
zone tables, the correlation view).

## Why build on it

- **The whole path, not just parsing.** Files in; normalized, validated,
  interpreted domain objects out — no re-implementing LAS mnemonic aliasing, unit
  harmonisation, petrophysical cutoffs, or surface gridding further up the stack.
- **A substrate, not a grab-bag.** Load a project once into a `GeoData` and
  operations broadcast across the whole collection. Immutable, strictly layered,
  fluent.
- **Values know what they are.** Reductions come back as a `Stats` bundle in
  canonical units; undefined samples are `NaN` (arithmetic propagates it, stats
  skip it).

## The `GeoData` substrate

Everything hangs off a `GeoData` project. Construct it with a length unit
(`"m"` or `"ft"` — petekIO never guesses), load once, then read interpreted
results.

```python
import petekio

geo = petekio.GeoData(unit="m")
geo.load_surface("top_res", "surfaces/top_res.irap")
geo.load_well("25/1-1", files="wells/25_1-1/")   # a folder of .wellpath + .las
geo.load_well_tops("field.tops")                  # Petrel tops → matching well + bore
```

A well folder ingests every `*.wellpath` (one per bore) and every `*.las` (logs),
auto-routing sidetracks; a vertical trajectory spanning the logged MD is built when
no survey is supplied. Named access (`geo.well(id)`, `geo.surface(name)`) and a
broadcastable `geo.wells` view come for free.

## Surfaces

A `Surface` is a regular gridded layer (e.g. a depth horizon) on a `GridGeometry`.
Operations return **new** surfaces — the type is immutable.

```python
top = geo.surface("top_res")
top.sample(cx, cy)              # NaN-aware bilinear point read (None outside the grid)
top.edge                        # convex edge polygon over defined nodes
top.geometry.edge               # same edge on the returned geometry object
top.stats().mean                # count / mean / min / max / std / p10 / p50 / p90
top.area_below(1990.0)          # Σ cell-area where value ≤ depth — the GRV-style query
top.resample(target_geometry)   # bilinear onto another lattice
base = geo.surface("base_res")
thick = petekio.Surface.thickness(top, base, clamp_zero=True)  # base − top, ≥ 0
```

Scattered `(x, y, z)` data grids into a surface via `PointSet.to_surface(geom,
method)` with `"nearest"`, `"idw"`, or `"minimum_curvature"` (Briggs biharmonic,
honouring the points as hard constraints).

If a point cloud is really a regular grid export, `PointSet.infer_geometry(...)`
can recover the lattice and attach an edge polygon:

```python
geom = pts.infer_geometry(tolerance=1e-3, edge="convex_hull")
surf = pts.to_surface(geom, method="nearest")
```

Inference is deliberately strict and raises when points are scattered, duplicate
onto the same lattice node, or do not fit the detected lattice within tolerance.

## Wells, logs and tops

A well carries a **trajectory** (MD ↔ position), **logs** (MD-indexed curves), and
**tops** (formation picks that define intervals). Per-bore access is first-class:

```python
w = geo.well("25/1-1")
w.bores()                       # e.g. ["", "A", "B"] — "" is the main bore
w.is_multibore                  # True → choose a bore before the top-level accessors
bore = w.sidetrack("A")
bore.xyz(2100.0); bore.tvd(2100.0)     # positioned by THIS bore's trajectory
bore.mnemonics()                        # curves present on the bore
bore.log("PHIE").stats().mean           # whole-curve stats; also .values() / .at_md(md)
```

A top resolves an interval `[top_md, base_md)` (base = the next top, or TD for the
deepest). The headline ergonomic is dynamic attribute access — interval → log →
`Stats` in one expression:

```python
w.set_default_bore("A")
w.upper_sand.phie.mean          # PHIE stats over the Upper Sand interval
w.top("Upper Sand")             # or the explicit Interval object
```

### Lithostratigraphic ordering

Zones come back in true stratigraphic order, not measured-depth order.
`load_well_tops` reads **every** well in the tops file and merges their relative
orderings into one field-wide column — so a marker that pinches out (zero
thickness) in one well is ordered correctly by a well that develops it. Where two
markers are coincident everywhere and the data can't order them, a soft
`strat_hint` ("A < B" = A above B) breaks the tie; real MD relationships always win
over a hint.

```python
geo.strat_order                 # the field-wide lithostratigraphic column
```

## Per-zone stats and `zone_table`

`zone_stats` gives per-zone `Stats` in stratigraphic order; `zone_table` assembles
a tidy pandas frame (`pip install petekio[pandas]`).

```python
bore.zone_stats("PHIE")                     # [(zone, Stats), ...] in lithostrat order
w.zone_table("PHIE", stats=("mean", "p50"))          # tidy [zone, bore, mean, p50]
w.zone_table("PHIE", pivot=True, decimals=3)          # wide: zone index × bore columns
w.zone_table("PHIE", aggregate=True)                  # pooled "all" row first per zone
w.zone_table("PHIE", stats=("mean", "gross", "samples"))  # gross = zone MD thickness
geo.wells.zone_table("PHIE")                          # multi-well; bore = "<well> <sidetrack>"
```

Averages are **thickness-weighted** by default — each sample counts for the depth it
represents, so a finely-sampled log can't outvote a coarse one (`weighted=False`
for a plain sample mean).

## Net cutoffs — `NetSettings`

`NetSettings` is the reservoir net/pay cutoff spec (φ / Sw / Vsh). Pass it to a
`net_zone_stats` call or a `zone_table` `cut=` to pool only the samples that pass —
the natural way to run an A/B sensitivity sweep between two cutoff scenarios.

```python
base = petekio.NetSettings(phi_min=0.08, sw_max=0.6)
strict = base.replace(phi_min=0.22, sw_max=0.4)         # a derived scenario spec
bore.net_zone_stats("PHIE", cut=base)                   # [(zone, Stats)] over NET samples
w.zone_table("PHIE", cut=strict, stats=("mean", "samples"))  # net-conditioned per cell
```

The curve names default to `PHIE` / `SW` (Vsh optional); a scenario is inert
without a `cut`.

## The correlation view

A well exposes a standalone log-correlation viewer that builds a bundle from the
well's own logs + trajectory and renders it (the viewer is an optional runtime
dependency, imported lazily). Serve it live, or export one self-contained HTML file:

```python
w.view()                                 # serve the logs (non-blocking) → a LogSession
w.view(curves=("PHIE", "SW"), tops=True) # select curves; include tops/zones
w.view(save="well.html")                 # export a self-contained HTML file instead
w.view(spec=petekio.ViewSpec(curves=("PHIE", "SW"), tops=True),
       settings=petekio.ViewSettings(save="well.html", serve=False))  # declarative
```

## Projects & persistence

A whole project serialises to a single structured `.pproj` file — atomic to write,
inspectable without a full load, and splittable / mergeable / tag-filterable.

```python
geo.save("field.pproj")                          # atomic whole-project write
petekio.GeoData.inspect("field.pproj")           # manifest dict: unit, owner, elements
geo2 = petekio.GeoData.open("field.pproj")        # materialize
petekio.GeoData.export("field.pproj", "share.pproj", ["field-a"])  # tagged subset
```

## Spec value-objects

The declarative, frozen load- and view-time specs — each is JSON-durable
(`to_dict` / `from_dict`), compares by value, and derives with `.replace(...)`:

- **`IngestSpec`** — load-time canonicalization (mnemonic `aliases`, `strat_hints`,
  `unit`), applied per call.
- **`NetSettings`** — the φ / Sw / Vsh reservoir cutoffs.
- **`ViewSpec`** / **`ViewSettings`** — *what* the correlation view shows and *how*
  it is delivered.

## Capabilities at a glance

| Domain | What you get |
| --- | --- |
| **Surfaces** | IRAP-classic / CPS-3 load, sample & resample (bilinear), edge polygons, arithmetic, stats, `area_below` volumetrics, gridding from scattered points (minimum-curvature) |
| **Wells** | Positioned `.wellpath` trajectories (MD preserved; minimum-curvature), multi-bore sidetracks, LAS logs with mnemonic aliasing, Petrel well-tops, per-zone stats, field-wide lithostratigraphic ordering, net cutoffs |
| **Points / polygons** | IRAP / GeoJSON / CSV load, strict regular-grid geometry inference, clip, point-to-surface gridding |
| **Project** | `GeoData` substrate — load once, broadcast across the collection; read-only filtered views; single-file `.pproj` persistence |

## Where to go next

- Run the two notebooks under `examples/notebooks/`.
- **API.md** — the locked public API contract (Rust, mirrored in Python).
- **SPEC.md** — the design constitution + architecture.
