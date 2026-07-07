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

## PointSet & PolygonSet

| Member | Description |
| --- | --- |
| `PointSet.bbox` / `.infer_geometry(...)` / `.to_surface(grid_geom)` | Bounds; strict regular-grid geometry inference; grid points onto an explicit geometry. |
| `GridGeometry.edge` | Edge polygon carried by inferred geometry, or a rectangular footprint for plain geometries. |
| `PolygonSet.rings` | The constituent rings. |

## Trajectory (standalone)

`petekio.Trajectory.from_stations([(md, inc_deg, azi_deg), ...], head=(x, y),
kb=...)` builds a minimum-curvature trajectory without a project; `.tvd(md)`,
`.xyz(md)`, `.md_at_tvd(tvd)`, `.md_range()`.
