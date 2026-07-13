# petekIO guide

petekIO is the subsurface **data layer**: a Rust library (with thin PyO3 bindings)
that turns raw subsurface files into clean, validated, interpreted domain objects
— **surfaces, wells (trajectories / tops / logs), points, and polygons**. The
application on top stays thin because petekIO does the unglamorous groundwork once,
behind a stable API.

The pipeline is the point:

**ingest → normalize → validate → interpret → characterise**

Raw reader quirks are normalized before they reach the public objects. Project
save/load persists the standard object graph — surfaces, points, polygons, wells,
and logs — rather than the original vendor format details.

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
thick = top.thickness(base, clamp_zero=True)  # base − top, ≥ 0
# Equivalent unbound form: petekio.Surface.thickness(top, base, clamp_zero=True)
top.thickness = thick              # add/replace a COW attribute lane
top.attr["thickness"]              # read the promoted lane as a Surface
smoothed = top.smooth(radius=1)    # moving average; holes remain holes
dip = top.dip_angle()              # degrees from horizontal
azimuth = top.dip_azimuth()        # down-dip, clockwise from North
filled = top.extrapolate("nearest")  # fill holes; also "idw" / "min_curvature"
```

Surface assignment accepts only another `Surface` with exactly the same grid
geometry (including origin, increments, node counts, rotation, and y-flip).
`set_attr("thickness", thick)` is the explicit equivalent. Attribute lanes are
read through `surface.attr[...]`, leaving `Surface.thickness(...)` callable as
the equivalent unbound thickness calculation; `surface.thickness(base)` remains
the normal instance form.

Dip uses central differences where both neighbours exist and one-sided
differences at boundaries or beside holes. Derivatives are transformed through
the grid spacing, rotation, and y-flip into world East/North. A flat node has
zero dip angle and undefined (`NaN`) azimuth. Extrapolation uses only finite
nodes as controls and changes only original `NaN` nodes; existing finite and
infinite values remain bit-for-bit unchanged. All four methods above return a
detached, same-geometry surface containing only its new primary lane.

Scattered `(x, y, z)` data grids into a surface via `PointSet.to_surface(geom,
method)` with `"nearest"`, `"idw"`, or `"minimum_curvature"` (Briggs biharmonic,
honouring the points as hard constraints).

If a point cloud is really a regular grid export, `PointSet.infer_geometry(...)`
can recover the lattice and attach an edge polygon. EarthVision/Petrel exports
with `column` and `row` fields can instead become a `StructuredMeshSurface`,
which keeps logical topology but stores explicit XY per node. Plain IRAP/XYZ
point exports have to infer from XY alone unless `Project.import_data(...)` can
enrich them from a same-stem EarthVision topology export in the raw project tree.

```python
geom = pts.infer_geometry(tolerance=1e-3)  # GridGeometry or bridged TriSurface fallback
if isinstance(geom, petekio.GridGeometry):
    surf = pts.to_surface(geom, method="nearest")
mesh = pts.to_structured_surface(edge="occupied")
```

Regular inference is deliberately strict. When the points do not fit a lattice,
`infer_geometry(...)` returns a `TriSurface` instead of inventing a geometry;
topology-bearing curvilinear exports can also be promoted with
`to_structured_surface(...)`, which stores explicit per-node XY.

When a surface export has lost its `column`/`row` fields, recover them rather than
forcing the points onto a lattice:

```python
labelled, report = pts.detect_topology()
if report.verified:
    mesh = labelled.to_structured_surface()   # exact: no point moved
else:
    tin = pts.to_tri_surface()                # fault-cut: report.blocks > 1
```

`detect_topology` detects the grid azimuth and a step per axis, then walks the grid
paths to *label* each point. It never moves one, and it will not walk across a fault:
there the neighbour relation is not determined by geometry, so it re-seeds and labels
the far side as its own **block** rather than silently welding them together.
`report.blocks == 1` means an uninterrupted grid; more means the surface is fault-cut,
and `verified` is `False`.

`to_tri_surface(max_link=None, max_bridge=None)` is the strict primitive for that case: the
points become the vertices of a triangulated surface, unmoved, and the fault is
honoured rather than bridged — `TriSurface.components` reports how many blocks
survived. `max_link` is the longest triangle edge to keep, in **cells**, and must lie
in `(√2, 2)`. `max_bridge` (also in cells, `>= max_link`) opt-in closes the mesh where
the geometry does not close — the boundary fringe, fault seams, interior data gaps —
admitting edges up to that length. The higher-level `infer_geometry(...)` fallback
defaults `max_bridge` to `3.4` cells to close ordinary export fringes and seams; pass
`max_bridge=None` there for the strict lattice-closed result. Calling
`to_tri_surface()` directly remains strict by default.

Geometry is a **flat empty shell** in three levels of complexity — the rigid
`GridGeometry` (eight scalars, XY computed), the `StructuredShell` (`(i, j)` nodes
with explicit XY), and the `MeshShell` (node ids + triangles + a quad-dominant
wireframe) — and surfaces are a shell plus per-node property lanes (`values` = z
first among equals, named attributes via `attr`/`set_attr`; shells are shared, so
extra attributes never repeat geometry). Every surface level offers
`iso_lines(interval=..., levels=..., attr=...)` contour polylines and
`value_layer(attr=...)` for value-coloured viewing (the petektools 2-D viewer
consumes both via `view2d(color=..., contours=...)`), and conversions run up the
ladder losslessly (`to_structured_mesh()`, `to_tri_surface()`) or down by
inference/resampling (`infer_grid(tolerance)`, `resample(geom, method)`).

The default `edge="full_rect"` is the four-corner rectangle of the inferred
lattice: cheap, but it claims the whole bounding lattice even where nodes carry no
data. Use `edge="occupied"` for the true data footprint — the outline of the
occupied nodes, which follows interior holes and a non-rectangular boundary, and
costs the same as `full_rect`. `edge="convex_hull"` gives a convex envelope for
comparison. Use
`to_structured_surface(...)` for topology-bearing Petrel surfaces that are
locally shifted around faults; use `to_surface(...)` when you want gridding onto
an explicit regular model geometry.

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
Use `Project.import_data(...)` for raw source trees; use `Project.load(...)` and
`Project.save(...)` only for compact `.pproj` files.

```python
project = petekio.Project.import_data("Data", settings=petekio.ImportSettings(crs="EPSG:32631"))
project.save("field.pproj")                      # atomic compact-project write
project = petekio.Project.load("field.pproj")    # compact-project read

project.rename_surface("Top reservoir", "structure/top dome")
project.surfaces                                 # ["structure/"]
project.surfaces.structure                       # ["top dome"]
project.surfaces.top_dome                        # unique leaf lookup
project.surfaces.all_names()                     # ["structure/top dome"]
project.delete_surface("structure/top dome")

geo.save("field.pproj")                          # lower-level GeoData write
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
| **Wells** | Positioned `.wellpath` trajectories (MD preserved; minimum-curvature), multi-bore sidetracks, imported logs stored as MD/value pairs with mnemonic aliasing, Petrel well-tops, per-zone stats, field-wide lithostratigraphic ordering, net cutoffs |
| **Points / polygons** | IRAP / GeoJSON / CSV load, strict regular-grid geometry inference, clip, point-to-surface gridding |
| **Project** | `GeoData` substrate — import raw data once, broadcast across the collection; read-only filtered views; compact `.pproj` load/save |

## Where to go next

- Run the two notebooks under `examples/notebooks/`.
- **API.md** — the locked public API contract (Rust, mirrored in Python).
- **SPEC.md** — the design constitution + architecture.
