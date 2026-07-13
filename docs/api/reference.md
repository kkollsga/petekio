# API reference

This mirrors the locked
[`API.md`](https://github.com/kkollsga/petekio/blob/main/API.md) contract — the
canonical source. Rust is canonical; the Python surface shown here marshals to
it one-to-one.

## GeoData

The load-once project substrate.

| Member | Description |
| --- | --- |
| `GeoData(unit="m")` | New project; `unit` is `"m"`/`"metres"` or `"ft"`/`"feet"`. |
| `.unit` | The project length unit (`"m"` / `"ft"`). |
| `.load_surface(name, path)` | Load an IRAP-classic surface under `name`; returns an owned `Surface`. |
| `.load_well(id, head=None, kb=None, files=...)` | Load a well from a directory/file; `head`/`kb` optional when a `.wellpath` supplies them. `files` is required. Returns a `Well` view. |
| `.load_well_tops(path)` | Load a multi-well Petrel tops file (Horizon picks → matching well + bore). Derives `strat_order` across the whole file. Returns the count assigned. |
| `.strat_order` | The global lithostratigraphic column (top names, shallow→deep) from the last `load_well_tops`; `[]` before any tops. |
| `.surface(name)` / `.points(name)` / `.polygons(name)` / `.well(id)` | Named access (`None` if absent). |
| `.surfaces()` | All surfaces, insertion order. |
| `.wells` | A broadcastable, filterable `WellsView`. |

## Well

A view into the project's well collection. {#well}

| Member | Description |
| --- | --- |
| `.id` | Well identifier. |
| `.head` / `.kb` / `.crs` | Wellhead `(x, y)`, kelly-bushing datum, CRS label (provenance only). |
| `.bores()` | Bore labels, in order (`""` is the main bore). |
| `.sidetrack(label)` / `.sidetracks()` | A bore by label / all bores. |
| `.xyz(md)` / `.tvd(md)` / `.md_at_tvd(tvd)` | Geometry on the main/active bore. |
| `.top(name)` | The `Interval` a top names, or `None`. |
| `.log(mnemonic)` | A full-curve `LogView`, or `None`. |
| `w.<top>.<log>` | Dynamic chain: `w.brent.ntg` → that top-interval log's `Stats`. |

## Sidetrack

A single bore (the real data lives on the named bores).

| Member | Description |
| --- | --- |
| `.label` | Bore label. |
| `.mnemonics()` | Curve names, insertion order. |
| `.tvd(md)` / `.xyz(md)` / `.md_range()` | Per-bore geometry. |
| `.log_stats(mnemonic)` | Whole-bore NaN-skipping `Stats`, or `None`. |
| `.zones()` | `[(name, top_md, base_md), ...]` in lithostratigraphic order. |
| `.zone_stats(mnemonic)` | `[(name, Stats), ...]` in lithostratigraphic order. |
| `.zone_stats(mnemonic, zone)` | That single zone's `Stats`, or `None`. |

## Interval & LogView

| Member | Description |
| --- | --- |
| `Interval.name` / `.top_md` / `.base_md` / `.thickness_md()` | The named interval. |
| `Interval.log(mnemonic)` | The log clipped to this interval (`LogView`), or `None`. |
| `interval.<log>` | Dynamic: `interval.ntg` → that log's `Stats`. |
| `LogView.stats()` / `.values()` / `.md()` / `.at_md(md)` | Stats, samples, depths, interpolated value. |

## Stats

NaN-skipping summary of a sample set: `.count`, `.mean`, `.sum`, `.min`, `.max`,
and percentiles (`.p10`, `.p50`, `.p90`).

## Surface

| Member | Description |
| --- | --- |
| `.geometry` / `.bbox` / `.edge` | `GridGeometry`; bounding box; convex edge polygon over defined nodes. `surface.geometry.edge` matches `surface.edge`. |
| `.stats` | NaN-skipping statistics. |
| `.sample(x, y)` | Bilinear sample at a world coordinate. |
| `.resample(grid_geom)` | Bilinear onto another geometry → new `Surface`. |
| `.area_below(z)` | Planimetric area below a depth. |
| operators | `+ - * /` with a scalar or a matching-geometry surface (elementwise). |

## StructuredMeshSurface

| Member | Description |
| --- | --- |
| `.kind` | Always `"structured_mesh"`. |
| `.ncol` / `.nrow` | Logical column/row node counts. |
| `.node_xy(i, j)` / `.z(i, j)` | Explicit node coordinate and primary value at logical node `(i, j)`. |
| `.values()` / `.x()` / `.y()` | Row-major nested lists for the explicit node arrays. |
| `.edge` / `.bbox()` | Modelling edge polygon and finite-node bounding box. |
| `.nominal_geometry` | Optional approximate `GridGeometry`; metadata only. |
| `.stats()` / `.history()` | NaN-skipping value statistics and operation history. |

## PointSet & PolygonSet

| Member | Description |
| --- | --- |
| `PointSet.bbox` / `.infer_geometry(...)` / `.to_surface(grid_geom)` | Bounds; strict regular-grid geometry inference with default `full_rect` point edge, returning a `TriSurface` with a 3.4-cell fallback bridge when no regular lattice fits (`max_bridge=None` opts out); grid points onto an explicit geometry. |
| `PointSet.to_structured_surface(...)` | Promote topology-bearing points (`column`/`row`) to `StructuredMeshSurface` while preserving explicit shifted XY nodes. |
| `PointSet.detect_topology(nominal_cell=None)` | Recover `column`/`row` from bare XYZ without moving a point. Returns `(points \| None, TopologyReport)`; `.verified` gates the labels, `.blocks > 1` means fault-cut. |
| `PointSet.to_tri_surface(max_link=None, max_bridge=None)` | The strict primitive when topology cannot be verified: a `TriSurface` over the original points, honouring faults rather than bridging them by default. `max_link` is in cells, in `(√2, 2)`; an explicit `max_bridge` opts into closing short fringes/seams. |
| `TriSurface.points()` / `.xyz()` / `.triangles()` | Original vertices (`xyz()` is the generic `view2d` protocol) and unstructured triangle indices. |
| `StructuredMeshSurface.to_points()` | Exact inverse of `to_structured_surface(...)` — copies node XY/Z, never resamples. |
| `PointSet.x` / `.y` / `.z` / `.<attr>` | Column objects for same-point-set calculations; assign with `points.new_attr = ...`. |
| `GridGeometry.edge` | Edge polygon carried by inferred geometry, or a rectangular footprint for plain geometries. |
| `PolygonSet.rings` | The constituent rings. |
| `PolygonSet.area` / `.<attr>` | Per-polygon column objects; `polygons.area()` remains total area, `polygons.total_area()` is explicit. |

## Calculated logs

| Member | Description |
| --- | --- |
| `project.wells.assign_log(name, expr)` | Assign a calculated log across wells/bores. Strict by default: log operands must share MD sampling. |
| `basis=logs.PHIE` | Output basis; other operands are resampled to PHIE using `interpolation=`. |
| `logs.NetSand.to_basis(logs.PHIE, interpolation="spline")` | Operand-local resampling. Interpolation: `nearest`, `linear`, `previous`, `next`, `spline` plus aliases. `spline` uses the shared `petektools` natural-cubic kernel when available. |

Imported LAS files do not remain calculation frames. A log's basis is its own MD
vector on the bore.

## Operation history

All value-bearing domain objects use the same underlying operation-history
container. Python exposes it as an ordered `list[str]` through `.history()`.
When an object is derived from another object, inherited entries are preserved;
secondary contributors are role-prefixed, for example `rhs.*`, `mask.*`, or
`prior.*`.

| Member | Description |
| --- | --- |
| `surface.history()` | Human-readable source/operation entries for surface loads, math, resampling, attributes, gridding, and clipping. |
| `points.history()` | Point-set load/create/filter/attribute history. Generated surfaces inherit the point history. |
| `polygons.history()` | Polygon load/create/attribute history. Clipped surfaces include the mask history. |
| `log.history()` | Log creation/assignment and view operations. Logs are stored as MD/value arrays; source files are not retained as calculation frames. |

## Trajectory (standalone)

`petekio.Trajectory.from_stations([(md, inc_deg, azi_deg), ...], head=(x, y),
kb=...)` builds a minimum-curvature trajectory without a project; `.tvd(md)`,
`.xyz(md)`, `.md_at_tvd(tvd)`, `.md_range()`.
