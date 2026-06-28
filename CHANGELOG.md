# Changelog

All notable changes to petekIO are recorded here. The format loosely follows
[Keep a Changelog](https://keepachangelog.com/); the project uses SemVer. The
`release` skill promotes `[Unreleased]` to a versioned block at release time.

## [Unreleased]

### Added
- `GeoData` (`manager`): the load-once project substrate. Named, insertion-
  ordered collections under one `Unit`; `new`, fluent loaders returning `&T`
  (`load_surface` IRAP classic; `load_points`/`load_polygons` extension-
  dispatched over the supported formats; `load_well` from a directory or single
  file — `*.las` → logs, tops `*.csv` (`name`,`md`) → tops on the main bore, with
  a vertical trajectory synthesized from the log MD span); named getters
  `surface`/`well`/`points`/`polygons` (miss → `None`); `surfaces()` iterator and
  `wells() -> WellsView`.
- `WellsView<'a>` (`manager`): a lightweight, broadcastable borrow over a
  project's wells (no cloning) — `filter(pred)`, `iter()`, and `tops(name)`
  (narrow to wells carrying that marker), plus `len`/`is_empty`. The substrate
  behind the per-well `Stats` broadcast.
- `PointSet` (`core`): scattered N×3 points with named `f64` attribute columns.
  Loaders `load_csv` (named X/Y/Z columns; other numeric columns → attributes)
  and `load_irap_points` (RMS plain `X Y Z`); ops `len`/`is_empty`/`filter`/
  `attr`/`stats`/`bbox`/`nearest` (rstar R*-tree, areal). Gridding
  `to_surface(geom, GridMethod)` with `GridMethod::{Nearest, InverseDistance,
  MinimumCurvature}` — Nearest + IDW (p=2, exact at data) are full; minimum
  curvature is a biharmonic (∇⁴z=0) SOR relaxation anchored at data nodes
  (interior 13-point stencil, near-edge 5-point harmonic fallback). New deps:
  `geo`, `rstar`.
- `PolygonSet` (`core`): polygon rings backed by `geo` predicates. Loader
  `load_irap_polygons` (RMS rings split on the `999.0` sentinel); ops
  `contains(x,y)` (point-in-polygon, **boundary-exclusive** per `geo`),
  `area()` (Σ `unsigned_area`), `bbox()`, and `clip(&Surface)` (masks nodes
  outside all polygons → `NaN`).
- Vector IO (`io`): GeoJSON + ESRI shapefile via `geozero` 0.15 —
  `PointSet::load_geojson` (a streaming `FeatureProcessor` carries each
  feature's **numeric** `properties{}` into attribute columns, NaN-filling the
  schemaless union; strings dropped), `PolygonSet::load_geojson`, and
  `PolygonSet::load_shapefile`. New dep: `geozero`
  (`with-geo`/`with-geojson`/`with-wkt`/`with-shp`).
- Well logs (`core`): `Log` (MD-indexed curve, `new`/`len`/`view`) and
  `LogView<'a>` — a borrowed-or-owned (`Cow`) window with `stats`,
  `stats_weighted` (element-wise PV-weighting), `filter`, `at_md` (linear
  interpolation), `resample(step)`, `values`/`md`. NaN = undefined throughout.
- Tops + intervals (`core`): `Top` (name + MD) and `Interval<'a>` (`log` clips a
  curve to `[top_md, base_md)`, `thickness_md`). Wired into `Sidetrack`
  (`add_log`/`add_tops`/`top`/`log`) and `Well` (delegating `top`/`log` to the
  main bore): tops sort by MD, the interval base is the next top's MD or total
  depth, enabling the `well.top("Brent")?.log("NTG")?.stats()` chain.
- IO (`io`): LAS reader via `las_rs` 0.2 (`Log::load_las` / `load_las_all`,
  NULL→NaN, shared index/MD curve) and a headered tops CSV reader via `csv`
  (`Top::load_csv`, name/MD columns by header). New deps: `las_rs`, `csv`.
- Well geometry (`core`): `Station` and the `TrajectoryInput` survey variants
  (`Xyz`/`MdIncAzi`/`Stations`/`Hold`/`Steer`), normalized to a positioned
  `Trajectory` via the **minimum-curvature** method (dogleg β + ratio-factor with
  a β→0 Taylor guard). `Trajectory` exposes `xyz`/`tvd`/`md_at_tvd`/`md_range`
  with linear interpolation and shallowest-crossing TVD inversion.
- `Well` → `Sidetrack` hierarchy (`core`): `Well::new`/`sidetrack`/
  `sidetrack_mut` (lazy bore creation)/`main`/`sidetracks`; `Sidetrack`
  `add_trajectory` (newest → active)/`set_active`/`active`/`trajectories`.
  `Well` and `Sidetrack` delegate `xyz`/`tvd`/`md_at_tvd` to the main/active
  trajectory. (Tops/logs deferred to Phase 4.)

## [0.1.0] - 2026-06-28

### Added
- Project scaffolding: single layered `petekio` crate
  (`foundation → io → core → analysis → manager → py`), MSRV 1.88, `py`/`f32`
  feature placeholders.
- `foundation` layer: `GeoError`/`Result`, `Unit` (+conversions), `Point3`,
  `BBox`, `GridGeometry` (rotation + `yflip`, with `node_xy`/`xy_to_ij`/`bbox`),
  and NaN-skipping `Stats` (`of`/`weighted`/`percentile`).
- `Surface` (`core`): construction (`new`/`constant`), attribute layers
  (`attr`/`set_attr`/`attr_names`/`as_attr_surface`), and **IRAP-classic** read/
  write (`load_irap_classic`/`save_irap_classic`).
- `Surface::sample` (strict NaN-aware bilinear) + `Surface::resample` onto a
  target geometry.
- `Surface` math (immutable, NaN-propagating): element-wise `ln`/`log10`/`exp`/
  `sqrt`/`abs`/`powf`/`clamp_min`/`clamp`; surface↔surface `plus`/`minus`/
  `times`/`divided_by` + `thickness`; operator overloads (scalar `+ - * /` →
  `Surface`, surface `+ - * /` → `Result<Surface>`).
- `Surface` statistics/volumetrics: `stats`, `area_below`/`area_above`,
  `volume_between`, and the `hypsometry` (area-vs-depth) curve.
- **Python bindings** (the `petekio` wheel, via PyO3/abi3 + maturin): a thin
  layer exposing `Surface` (`load_irap_classic`/`save_irap_classic`/`sample`/
  `area_below`/`area_above`/`stats`, `ncol`/`nrow`/`rotation_deg`) and `Stats`.
