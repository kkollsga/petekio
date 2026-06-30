# Changelog

All notable changes to petekIO are recorded here. The format loosely follows
[Keep a Changelog](https://keepachangelog.com/); the project uses SemVer. The
`release` skill promotes `[Unreleased]` to a versioned block at release time.

## [Unreleased]

### Added
- **Python multi-bore well surface** — `GeoData.load_well_tops(path)`; `Well.crs`
  / `Well.bores()` / `Well.sidetrack(label)` / `Well.sidetracks()`; and a
  `Sidetrack` binding with `mnemonics`/`log_stats`/`zones`/`zone_stats`/`tvd`/
  `xyz`/`md_range`. Python can now reach per-bore logs + per-zone stats (the data
  lives on the named bores, not the main bore).

### Fixed
- Petrel well-tops: capture the `Type` column; `GeoData::load_well_tops` now
  ingests only `Horizon` picks and skips `Other` (fluid contacts OWC/GOC/FWL),
  so derived zones are purely lithostratigraphic (contacts no longer split zones).

### Changed
- `GeoData::load_well` now walks a directory **recursively** (handles a Petrel
  export tree with separate `Paths/`/`Logs/` subdirs, not just a flat folder) and,
  when filenames carry the well id, **ingests only that well's files** (skips
  others sharing the tree). Flat per-well folders with generic filenames are
  unchanged. A LAS that fails to parse is now **skipped, not fatal**.

### Fixed
- LAS reader: fall back to the **first curve as the depth index** when the index
  mnemonic isn't the standard `DEPT` (e.g. Petrel core logs name it `DEPTH`).
- LAS/Petrel readers (`wellpath`, `petrel_tops`): **decode Latin-1/Windows-1252**
  exports (each byte → its Unicode code point) instead of erroring on non-UTF-8 —
  Norwegian names (`ø`/`å`/`æ`) now decode correctly rather than to `�`.

## [0.2.2] - 2026-06-29

### Added
- `analysis::normalize::canonical_mnemonic_with` + `NameMap::get` — resolve LAS
  mnemonics against a user alias map first (for the choices the table can't guess,
  e.g. `NTG_PhieLam` vs `NTG_VShale` → `NTG`), then the built-in table.

- **`.wellpath` ingest + multi-sidetrack wells** — `GeoData::load_well` reads
  Petrel `.wellpath` traces: one **bore per file** (labelled by filename stem),
  each a **positioned** trajectory (`TrajectoryInput::PositionedSurvey`, MD
  preserved, subsea `z = TVD − kb`); logs route to the matching bore. The
  wellhead XY / KB / **CRS** from the header are authoritative (`Well::crs`/
  `set_crs`; CRS recorded, never reprojected). No-`.wellpath` wells keep the
  synthesized-vertical behaviour.
- **Petrel well-tops ingest** — `GeoData::load_well_tops(path)` reads a multi-well
  `# Petrel well tops` export (quoted Surface/Well, `-999` nulls) and routes each
  pick to the matching loaded well + bore (`"99/9-1 B"` → well `99/9-1`, bore `B`).
- **Core data tagging** — `LogKind` (`Log` / `Core`) on `Log` (`kind()` /
  `with_kind`); `load_well` tags curves from `*core*.las` files as `Core` so
  consumers can include/exclude core in per-zone aggregation.
- **Per-zone aggregation** — `Sidetrack`/`Well` `zones()` (every formation zone
  as an `Interval`) and `zone_stats(mnemonic)` → per-zone `Stats` (average via
  `mean`, `sum`, percentiles). Broadcastable across a project's wells.

### Changed
- `canonical_mnemonic` now strips a trailing vintage tag (`PHIE_2025` → `PHIE`)
  and keeps **effective vs total water saturation distinct** (`SWT` no longer
  collapses to `SW`); unknown mnemonics pass through vintage-stripped (original case).

### Added
- `Trajectory::from_input` is now public — build a positioned path from a survey
  (`TrajectoryInput`) standalone, without a `Well`/`GeoData`.
- Python `Trajectory` binding: `Trajectory.from_stations([(md, inc, azi), …],
  head=(x, y), kb=…)` plus `xyz` / `tvd` / `md_at_tvd` / `md_range` — directional
  surveys can now be built and queried directly from Python.

### Added
- `PolygonSet::rings()` — exterior ring vertices per polygon (`[x, y, z]`, z=0,
  closed) + Python binding, so consumers can read the boundary outline geometry
  (not just `area`/`bbox`/`contains`).

### Changed
- New **`algorithms/`** layer: pure, type-light numeric kernels grouped by
  discipline (`algorithms::wells` — minimum-curvature survey: `tangent`,
  `dogleg`, `ratio_factor`, `arc_point`, `survey_positions`), with analytic QC
  tests. `Trajectory` now delegates to it (no behaviour change); the ratio-factor
  formula has a single home. See `SPEC.md` §9 (the algorithm discipline).

### Fixed
- `Trajectory::xyz`/`tvd` now interpolate along the **minimum-curvature arc**
  between survey stations (slerp of station tangents + partial dogleg) instead of
  straight-lining between station nodes. Mid-station TVD was previously off by up
  to ~40 m in build sections; it now matches an independent survey reference to
  <0.05 m. (`Xyz` paths still use straight-line interpolation.)

## [0.2.1] - 2026-06-29

### Added
- **Model-ready inputs contract (GATE-0)** — `GeoData::model_inputs() -> Result<ModelInputs>`,
  the locked seam consumers (petekSim) build to. Uncertainty/provenance
  vocabulary (`Uncertain`, `Distribution`, `Provenance`) and the contract types
  (`ModelInputs`/`SummaryInputs`/`SpatialInputs`/`HorizonInput`/`WellCurveInput`).
- `Uncertain` constructors: `hard`/`defaulted`/`assumed` (deterministic),
  `uniform`/`triangular`/`normal`/`lognormal` (characterised), `from_stats`, and
  the `with_provenance` builder.
- `Unit::area_to_acres` — planar area (m²/ft²) → acres, backing
  `reservoir_area_acres`.
- `Well::logs()`/`Well::mnemonics()` and `Sidetrack::logs()` — enumerate every
  curve on a bore (previously curves were only fetchable by mnemonic).
- `analysis::normalize` — input canonicalisation: `canonical_mnemonic` (LAS
  mnemonic alias table, unknowns pass through), `parse_length_unit`,
  `is_percent_unit`, `harmonise_fraction` (percent→fraction), `harmonise_length`,
  and `NameMap` (case-insensitive formation/well alias → canonical, identity for
  unknowns).
- `analysis::validate` — physical validity ranges per canonical mnemonic
  (`validity_range`, `in_range`) and `mask_out_of_range`, which rejects
  out-of-range samples to `NaN` (the undefined convention) and reports the count.
- `analysis::interpret` — petrophysical interpretation (petekIO owns net_pay):
  `Cutoffs` (φ/Sw/Vsh, defaulted 0.08/0.5/0.5), `net_flags` (per-sample
  reservoir/pay flag), `net_pay` (Σ Voronoi thickness over net samples, TVD
  depth), `net_to_gross`, and `leverett_j`.
- `analysis::characterise` — fit an `Uncertain` from a sample set:
  `DistributionShape` (Normal / Triangular = P10/P50/P90 / LogNormal) and
  `characterise`, collapsing to Deterministic below two defined values.
- `Surface::smooth` (NaN-aware moving-average, preserves the defined mask),
  `Surface::boundary_polygon` (convex hull of defined nodes), and
  `PointSet::regrid_min_curvature` (warm-started incremental min-curvature
  re-grid on a prior surface's lattice, honouring control points as hard
  constraints).
- **`GeoData::model_inputs()` implemented** — assembles the full
  normalize→validate→interpret→characterise pipeline across a project into the
  `ModelInputs` contract (summary scalars as `Uncertain`, horizons, canonical
  well curves, boundary). `GeoData::surfaces_named`/`polygons_named` accessors;
  `Surface` and `PolygonSet` are now `Clone`.
- `WellCurveInput.xyz` — each curve sample positioned to world `(x, y, z=TVD)`
  via the trajectory, so consumers can upscale logs onto grid cells without
  touching positioning (which is petekio's responsibility).

## [0.2.0] - 2026-06-28

### Added
- Python wheel (`py`, PyO3): grew the `petekio` bindings from the early
  Surface+Stats layer to mirror `API.md` §"Python (PyO3) surface".
  - `Surface`: `load_irap_classic`/`constant`/`save_irap_classic`, `sample`,
    `resample`, element-wise math (`ln`/`log10`/`exp`/`sqrt`/`abs`/`powf`/
    `clamp_min`/`clamp`), named surface↔surface forms (`plus`/`minus`/`times`/
    `divided_by`) and the `+ - * /` operator overloads (scalar **and**
    surface↔surface, raising on geometry mismatch; reflected scalar forms),
    `thickness` (staticmethod), `stats`/`area_below`/`area_above`/
    `volume_between`/`hypsometry`, attribute access via `surface.attr["name"]`
    /`surface.attr(name)` (promotes to a `Surface`) + `attr_names`/`set_attr`,
    and `geometry`/`ncol`/`nrow`/`rotation_deg`/`bbox` getters.
  - `GridGeometry` (constructable) and `BBox` value types; `Stats` fields stay
    read-only attributes with `percentile`.
  - Numpy/`ndarray` exposure is out of scope (no numpy dependency): attribute
    layers are returned as promoted `Surface`s, never raw arrays.
  - `PointSet`: `load_csv`/`load_geojson`/`load_irap_points` classmethods,
    `len`, `attr` (→ `list[float]`), `stats`, `bbox`, `nearest`, and
    `to_surface(geom, method)` with `method` a string (`"nearest"`/`"idw"`/
    `"min_curvature"`, IDW default).
  - `PolygonSet`: `load_geojson`/`load_irap_polygons`/`load_shapefile`,
    `contains`, `area`, `bbox`, `clip`.
  - `GeoData(unit="ft"|"m")`: `load_surface` (→ owned `Surface`),
    `load_points`/`load_polygons` (→ views), named getters
    `surface`/`points`/`polygons` (miss → `None`), `surfaces()`, and the `unit`
    getter. Points/polygons hand back lightweight views that re-resolve into
    the project; surfaces are deep-cloned owned copies.
  - `Well`/`Interval`/`LogView`: `GeoData.load_well`/`well`/`wells` plus
    `well.xyz`/`tvd`/`md_at_tvd`, `well.top(name)` → `Interval`,
    `well.log(mnemonic)` → `LogView`; `Interval.top_md`/`base_md`/`name`/
    `thickness_md`/`log`; `LogView.stats`/`values`/`md`/`at_md`/`len`. The
    headline dynamic chain: `w.brent` → `Interval` and `w.brent.ntg` /
    `w.brent.phie.mean` → `Stats` via `__getattr__` (unknown names fall back to
    `AttributeError`). Wells are views into the project.
  - `WellsView` broadcast (`geo.wells`): `filter(predicate)` (a Python callable
    over `Well`), `tops(name)`, `iter()`/`len`, and `__getattr__` broadcast —
    `geo.wells.tops("Brent").ntg` (or `geo.wells.brent.ntg`) yields a per-well
    `list[Stats]`.
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
