# petekIO — subsurface data model & IO library (build spec)

> Repo: `Koding/Rust/petekIO` · crate name: `petekio` (lowercase for
> crates.io — confirm availability). Sibling to `logSuite`. This is the handover
> spec for the build agent.

A standalone, reusable **Rust** library (with optional **PyO3** Python bindings)
that is the complete **input data model** for subsurface work: surfaces, wells
(trajectories/tops/logs), points, polygons — with loading, calculations,
interpolation, filters, and statistics built in. It fills a real gap (no Rust
crate does this; xtgeo/welly are Python-only) and is the data foundation that
apps like SimulatoRS consume so they do **zero** parsing/interpolation themselves.

**Litmus test for what belongs here:** it must be useful to anyone working with
subsurface data — a geologist, a seismic interpreter, a petrophysicist — **not
just a reservoir simulator.** Nothing reservoir-specific (wireframes, grids,
volumetrics) lives here; that's the consumer's job.

---

## Design constitution (adopted from logSuite)

1. **Strictly layered, one-way deps.** `foundation → algorithms → io → core →
   analysis → manager`. A layer imports only from below — never sideways, never
   up. (`algorithms` depends only on `foundation`; `io` and `algorithms` are
   siblings above it.)
2. **A manager substrate.** Load once into a `GeoData` project; everything reads
   from it. **No per-item loops** — operations broadcast across the collection.
3. **Domain objects carry their operations** (arithmetic, filters, interpolation,
   stats) as methods/traits — fluent and chainable.
4. **Views** = read-only filtered subsets (`project.wells.filter(...)`).
5. **Open/closed.** Extend by adding new readers/operations/artifacts, not by
   editing existing types.
6. **Compartmentalized — split the elephant.** One module/topic, one
   type/responsibility. Soft limits: module ≲600 lines, type ≲300, method ≲50.
7. **Minimal public surface.** Re-export only what users need.
8. **Rust core + thin PyO3.** All logic in Rust; bindings only marshal. The
   Python API mirrors the Rust API in the fluent logSuite style.
9. **Algorithms are isolated, QC-able, discipline-grouped kernels.** High-value
   numeric / geostatistical routines live in **`algorithms/`, grouped by
   discipline** (`wells`, `grids`, …) as **pure, type-light functions** —
   primitives + `foundation` types in/out, no domain-object (`Surface`/`Well`/…)
   or IO coupling. Domain types call into them; a kernel's math has **one home**
   (never duplicated across call sites). Rationale: each kernel is trivial to
   **QC in isolation** (analytic tests on raw numbers), and a kernel that proves
   high-value is a cheap **lift-and-shift into the external `petekTools`
   library** (the module mirrors its type-light boundary). Don't inline a formula
   in a domain type.

---

## Architecture

A Cargo workspace (or one crate with layered modules — the dep arrow must stay
scannable). Composes the author's own crates + the Rust geo ecosystem; does NOT
reinvent them.

```
petekio/
  foundation/   errors · units · geometry (Point2/Point3, BBox, GridGeometry)
  algorithms/   pure numeric kernels, grouped by discipline: wells (min-curvature survey) · grids (gridding) — type-light, QC-able, petekTools-offloadable
  io/           irap · zmap · csv · las (←las_rs) · excel (←calamine) · survey · tops · vector (←geozero)
  core/         Surface · Well→Sidetrack→Trajectory · Log · Top · PointSet · PolygonSet  (+ operation traits)
  analysis/     resample · statistics · filters · arithmetic · model-ready-inputs pipeline
  manager/      GeoData (the substrate) · Views
  py/           PyO3 bindings (optional feature)
```

**Dependencies (compose, don't reinvent):**
- `las_rs` (own) — LAS logs. `calamine` — Excel/tabular (`.xlsx`/`.xls`).
- `geo` / `geozero` — polygon/point IO (GeoJSON/shapefile/WKT) + 2-D predicates.
- `ndarray` — the array backbone for surfaces (efficient, contiguous, BLAS-ready).
- `rstar` — spatial index (nearest cell / point-in-set) when needed.
- `serde` — (de)serialization of the model.
- *(Phase 3, cubes only)* `giga-segy` — SEG-Y reader (rev2, active); `ibmfloat`
  fallback codec. ZGY/OpenVDS have no Rust → FFI, deferred.

**Greenfield (the genuinely missing parts):** the `Surface` type + IRAP/ZMAP
readers, the `Well/Sidetrack/Trajectory` model + minimum-curvature, surface ops,
resampling, the tops→interval→stats machinery, the `GeoData` substrate.

---

## Conventions

- **Coordinate system:** project (x=Easting, y=Northing, z=depth increasing
  **downward**), feet OR metres — carried as a `unit` on the project; the library
  never guesses, conversions live in `foundation/units`.
- **Undefined values:** `f64::NAN` in arrays (and a fast `is_defined` mask).
  Arithmetic propagates NaN; stats skip NaN.
- **Storage precision:** `f64` default; a `f32` feature/typedef for memory-bound
  large surfaces. Arrays are contiguous `ndarray::Array2`.
- **Errors:** one `GeoError` enum (`thiserror`); `Result<T, GeoError>` everywhere.
- **Immutability of ops:** arithmetic/resample return **new** objects; mutation is
  explicit (`set_*`).

---

## Core data model

### `Stats` — the universal aggregation result
Returned by every reduction (a surface's values, a log over an interval, a point
attribute). The thing `well.brent.ntg` evaluates to.

```rust
pub struct Stats {
    pub count: usize,
    pub mean: f64, pub min: f64, pub max: f64, pub std: f64, pub sum: f64,
    pub p10: f64, pub p50: f64, pub p90: f64,   // default percentiles
}
impl Stats { pub fn percentile(&self, p: f64) -> f64; }   // arbitrary p
// Built via a builder that supports WEIGHTING (by interval length, or by another
// curve, e.g. pore-volume-weighted Sw): Stats::weighted(values, weights).
```
Python: `s.mean`, `s.p50`, `s.percentile(0.25)`.

### `Surface` — a regular gridded surface (the workhorse)
A rotated regular grid (IRAP/RMS model) holding a primary value layer plus named
**attribute** layers (thickness, seismic, time, …) on the same geometry.

```rust
pub struct GridGeometry {           // regular, rotatable
    pub xori: f64, pub yori: f64,   // origin
    pub xinc: f64, pub yinc: f64,   // node spacing
    pub ncol: usize, pub nrow: usize,
    pub rotation_deg: f64,          // CCW from East
    pub yflip: bool,
}
pub struct Surface {
    pub geom: GridGeometry,
    values: Array2<f64>,                       // primary (e.g. depth); NaN = undefined
    attributes: IndexMap<String, Array2<f64>>, // thickness, seismic_amp, twt, ...
}
```

**Required behaviour (maps to the requested features):**
- **Load:** `Surface::load_irap_classic(path)` — **FIRST format to implement.**
  (Then `load_irap_binary`, `load_zmap`, `load_xyz_csv`.)
- **Attributes:** `surface.attr("thickness") -> &Array2` ; `set_attr(name, arr)` ;
  `as_attr_surface("seismic") -> Surface` (promote an attribute for ops).
- **Scalar arithmetic + math:** operator overloads (`&s + 100.0`, `&a * &b`) and
  methods on the active layer: `.ln() .log10() .exp() .powf(n) .sqrt() .abs()
  .clamp_min(x) .clamp(lo,hi)`. Each returns a new `Surface`.
- **Surface↔surface:** `a.minus(&b)`, `a.times(&b)`, … require equal geometry
  (else `GeoError::GeometryMismatch` — caller resamples first). Convenience:
  `Surface::thickness(top, base, clamp_zero: bool) -> Surface`.
- **Statistics:** `surface.stats() -> Stats` (over defined nodes).
- **Interpret / repair:** `smooth(radius)` is a NaN-mask-preserving moving
  average; `dip_angle()` and `dip_azimuth()` use NaN-aware finite differences
  transformed from lattice axes into world East/North; `extrapolate(method)`
  fills only original NaNs from finite nodes through the shared nearest, IDW,
  or minimum-curvature grid kernel. These return same-geometry, primary-only
  surfaces and append operation history.
- **Area / volume:** `surface.area_below(depth) -> f64` (areal extent of nodes
  with value ≤ depth × cell area — the GRV-style query); `area_above`,
  `volume_between(&other) -> f64`, `hypsometry()` (area-vs-depth curve).
- **Resample (native):** `surface.resample(&target: GridGeometry) -> Surface`
  (bilinear; NaN-aware). `target` may come from another surface or a grid's areal
  geometry. Also `sample(x, y) -> Option<f64>` (point query, bilinear).

Python ergonomics:
```python
top  = geo.load_surface("top.irap")
base = geo.load_surface("base.irap")
thick = top.thickness(base, clamp_zero=True)                    # normal instance form
petekio.Surface.thickness(top, base, clamp_zero=True)           # equivalent unbound form
top.thickness = thick                    # assignment sugar for top.set_attr("thickness", thick)
top.attr["thickness"]                    # promoted attribute Surface; exact geometry required
trend = top.attr("seismic").ln()
top.smooth(radius=1)                     # preserves the original NaN mask
top.dip_angle(); top.dip_azimuth()       # degrees; azimuth clockwise from North
top.extrapolate(method="nearest")        # fills NaNs only; idw/min_curvature too
top.stats.p50
top.area_below(8240)                 # ft² below the OWC
ongrid = top.resample(grid_geom)     # bilinear onto a target geometry
```

Python `Surface` attribute assignment is typed: the right-hand side must be a
`Surface` with identical complete `GridGeometry` (origin, increments, node
counts, rotation, and `yflip`). Assignment adds or replaces a copy-on-write
attribute lane; read it through `surface.attr[name]`, so a lane named
`thickness` does not shadow either the normal `surface.thickness(base)` instance
form or the equivalent unbound `Surface.thickness(surface, base)` form.

### Geometry shells — the three-level system (level 2/3 surfaces)

Geometry is a **flat empty shell**: purely topological/positional, never a
function of z. Three levels of increasing complexity, matched to how far a
real export departs from a regular grid:

1. **`GridGeometry`** (rigid grid): 8 scalars, node XY computed. `Surface` =
   this + value lanes (above).
2. **`StructuredShell`**: `(i, j)`-organized nodes with explicit per-node XY
   (fault-shifted / curvilinear Petrel meshes that keep rectangular logical
   topology). `StructuredMeshSurface` = shell + primary values + attribute
   lanes.
3. **`MeshShell`**: integer node ids, 2-D XY, CCW triangles, quad-dominant
   wireframe, boundary edge, per-node walk labels `(block, i, j)`.
   `TriSurface` = shell + per-node z + attribute lanes (the fault-cut
   fallback).

Rules: shells are **immutable** once built and shared via `Arc` (N property
lanes never repeat geometry in memory); conversions go **up for free**
(lossless, node identity preserved, all attribute lanes carried 1:1) and
**down by inference** (`infer_grid` fits a regular lattice or refuses;
`resample` grids every lane through the shared gridding kernels). Derived
walkability (the `MeshShell` corner table) is lazy and never serialized.
Every level exposes `iso_lines` (NaN-aware marching triangles; holes break
lines, never bend them) and `value_layer` (the viewer's trimesh bundle).
`.pproj` stores a level-2/3 surface's shell once with N property lanes.

Python `PointSet.infer_geometry(...)` returns **only empty geometry roles**:
`GridGeometry` when the points fit one affine lattice; `StructuredShell` when
validated explicit `column`/`row` topology describes a curvilinear mesh; or
`MeshShell` when the existing fault-aware triangulation is required. It never
returns `Surface`, `StructuredMeshSurface`, or `TriSurface`; values remain on
the `PointSet` until an explicit `to_surface`, `to_structured_surface`, or
`to_tri_surface` call. Mesh fallback retains `max_bridge=3.4` by default and
strict `None`; `fallback="error"` remains fatal. `fallback="tri"` is a
deprecated compatibility spelling of `fallback="mesh"`, not a request for a
value-bearing `TriSurface`.

### `Well` → `Sidetrack` → `Trajectory` (+ tops + logs)
```rust
pub struct Well {
    pub id: String,
    pub head: (f64, f64),            // wellhead x, y
    pub kb: f64,                     // KB elevation / air gap — the MD datum
    sidetracks: IndexMap<String, Sidetrack>,  // "" = main (default), "a","b",...
}
pub struct Sidetrack {
    pub label: String,
    trajectories: Vec<Trajectory>,   // a sidetrack may hold several survey versions
    active: usize,                   // exactly one is active
    logs: IndexMap<String, Log>,     // assigned to THIS sidetrack
    tops: Vec<Top>,                  // assigned to THIS sidetrack
}
pub enum TrajectoryInput {           // construction modes, all normalized internally
    Xyz(Vec<[f64;3]>),               // explicit positions
    MdIncAzi(Vec<(f64,f64,f64)>),    // survey stations → minimum-curvature
    Stations(Vec<Station>),          // raw survey stations
    Hold { from: Station, to_md: f64 },        // constant inc/azi segment
    Steer { from: Station, build: f64, turn: f64, to_md: f64 },  // build/turn rates
}
pub struct Trajectory { /* normalized */ path: PositionedPath }  // md → (x,y,z,tvd)
pub struct Top { pub name: String, pub md: f64 }   // entry MD; interval base = next top's MD (or TD)
pub struct Log { pub mnemonic: String, md: Array1<f64>, values: Array1<f64>, unit: String }
```

**Required behaviour:**
- **Construction:** `Well::new(id, head, kb)`; `well.sidetrack_mut(label)` (creates
  `""`/main lazily); `st.add_trajectory(TrajectoryInput) -> &mut Trajectory`
  (newest becomes active unless told otherwise); `st.set_active(i)`.
- **Trajectory types:** every `TrajectoryInput` variant normalizes to a
  `PositionedPath` — `MdIncAzi`/`Stations` via **minimum-curvature** (dogleg ratio
  factor), `Xyz` directly, `Hold`/`Steer` by integrating the segment. From the
  wellhead `head` + datum `kb`.
- **Interpolation (native):** on the active trajectory, exposed on the well:
  `well.xyz(md) -> [f64;3]`, `well.tvd(md) -> f64`, `well.position_at(md)`,
  `well.md_at_tvd(tvd)`. Arc/linear interpolation between stations.
- **Surface intersections (native):** `Trajectory`/`Sidetrack`/resolved `Well`
  intersect regular, structured, and triangulated surfaces through one canonical
  triangle kernel. It spatially narrows candidates, adaptively subdivides the
  actual minimum-curvature path, refines roots/tangencies, de-duplicates shared
  edges, skips null holes/outside geometry, and rejects coplanar overlap loudly.
  `intersections` returns every MD-ordered hit; `intersection` returns `None` for
  no hit and errors with guidance if more than one exists. Computation is pure.
- **Explicit persistent picks:** a bore/resolved well may add, replace, or remove
  a top from an MD or typed intersection. Duplicate/missing/ambiguous legacy
  names fail; hit well/bore identity and recomputed XYZ must match. Persistence
  remains exactly `Top { name, md }` inside the owning Well.
- **Logs (native):** `well.log("PHIE")` → a `LogView` with `.filter(pred)`,
  `.resample(step)`, `.stats() -> Stats`, `.values()`, `.at_md(md)`. Positioned in
  3-D via the active trajectory (`.xyz()`).
- **Tops → interval → stats (native, the key ergonomic):** a top defines a depth
  **interval** (its MD → the next top's MD). `well.top("Brent")` → an `Interval`;
  `interval.log("NTG").stats() -> Stats`. The headline Python ergonomic is dynamic
  attribute access:
  ```python
  w.brent.ntg            # -> Stats(mean,min,max,p10,p50,p90,std) over the Brent interval
  w.brent.phie.mean
  w.xyz(2450)            # interpolated position at MD 2450
  ```
  (Rust: `well.top("Brent")?.log("NTG")?.stats()`. PyO3 `__getattr__` resolves
  `.<top>` then `.<log>`.)

### `PointSet` (simpler)
```rust
pub struct PointSet { coords: Array2<f64> /* N×3 */, attributes: IndexMap<String, Array1<f64>> }
```
- Load: xyz/CSV, IRAP points, GeoJSON (`geozero`). Ops: `filter(pred)`,
  `attr(name)`, `stats(attr) -> Stats`, `bbox()`, `nearest(x,y)` (`rstar`),
  `to_surface(geom, method)` (grid scattered points → a `Surface`).

### `PolygonSet` (simpler)
```rust
pub struct PolygonSet { polygons: Vec<Polygon /* geo::Polygon + optional z */> }
```
- Load: GeoJSON/shapefile/WKT (`geozero`), IRAP polygons. Ops: `contains(x,y)`,
  `area()`, `clip(&surface)`, `bbox()`. Used for boundaries + (later) fault traces.

### `Cube` — 3-D regular volume (seismic & inversions) — *designed, deferred*
A seismic survey or an inversion result is a 3-D regular volume: a rotatable
(inline × crossline) lattice with a vertical axis (time or depth). An inversion
(acoustic impedance, Vp/Vs, a porosity cube) is the **same type** with a
different value — like `Surface` attributes but in 3-D. (Design distilled from
Equinor `xtgeo`/`segyio`/`open-zgy`/`OpenVDS` + the Rust `giga-segy` crate.)

**Geometry — xtgeo's model + one improvement.**
```rust
pub struct CubeGeometry {
    pub areal: GridGeometry,         // inline/xline (reuses the surface geometry: origin/inc/rotation/yflip)
    pub zori: f64, pub zinc: f64, pub nz: usize,
    pub ilines: Vec<i32>, pub xlines: Vec<i32>,  // explicit line LABELS (segy access is by label, not index)
    pub domain: Domain,              // Time | Depth   (xtgeo omits this — we keep it explicit)
}
pub struct Cube {
    pub geom: CubeGeometry,
    pub property: PropertyKind,      // Amplitude | AcousticImpedance | VpVs | Porosity | ...  + unit
    store: CubeStore,                // dense (small) OR brick-tiled (large) — see below
    dead: Bitmask,                   // dead/undefined traces (xtgeo's traceidcodes)
}
```
One `Cube` carries seismic **and** inversion alike; the *meaning* lives in
`property`/unit metadata (the improvement over xtgeo, which leaves `values`
semantics-agnostic).

**Storage — the key architecture decision (do NOT copy xtgeo's whole-cube-in-RAM).**
xtgeo holds the entire `(ncol,nrow,nlay)` f32 array in memory — fine for small
cubes, the **ceiling to beat** for GB–TB volumes. ZGY / OpenVDS / seismic-zfp all
converge on the same answer, which we adopt:
> **Store the cube as a grid of fixed-size compressed bricks (~64³ samples) plus
> an offset lookup table — never a naïve contiguous N-D array.** A slice /
> sub-volume / horizon-extraction query then fetches only the few intersecting
> bricks (cheap random access, cloud-friendly ranged reads, per-brick
> compression, optional LOD pyramid).
`CubeStore` is an enum: `Dense(Array3<f32>)` for small cubes; `Bricked { bricks,
offset_table, .. }` for large. The `CubeGeometry` is the thin addressing layer
over the bricks. (Start with `Dense` + a seismic-zfp-style ZFP brick model;
full VDS/LOD is later.)

**IO — depend on `giga-segy`, do not reimplement SEG-Y from scratch.**
SEG-Y is a public standard (no IP upside to re-deriving) and quirk-heavy (IBM
floats, endianness, **configurable byte locations** — 189/193 default, never
assume — headers, rev1/rev2). The Rust crate **`giga-segy`** (GiGainfosystems,
rev2, active, permissively licensed, `-core`/`-in`/`-out` split that already matches our
layering) covers it. Wrap it behind a `SegyReader` trait in `io/`. **Caveat:
verify IBM-float (format code 1) round-trips on a real file before committing;**
`ibmfloat` crate as a fallback codec. Mirror segyio's two-layer split (lazy
trace reader + geometry/format detection) and **lazy/streaming by default**,
mmap opt-in. ZGY / OpenVDS have **no Rust** (C++/Python) → FFI, deferred.

**The link to the rest (the valuable op):** `surface.slice_cube(&cube, sampling)`
samples a cube along a horizon → a **surface attribute** (amplitude/impedance on
the top); `slice_cube_window(min|max|rms|mean over a z-window)` for windowed
attributes (xtgeo's `slice_cube_window`). This *is* the "seismic trend on a
surface" the MVP already supports as a surface attribute — the cube is just the
full-volume source. Also `cube.sample(x,y,z)`, `cube.inline(label)/zslice(z) ->
Surface`, `cube.stats()`, `cube.resample(&CubeGeometry)`.

**Domain conversion** (time↔depth via a velocity model) is heavier — deferred.

### `GeoData` — the manager substrate
Load everything once; named + collection access; views; broadcast.
```rust
pub struct GeoData { unit: Unit, surfaces: IndexMap<String, Surface>,
                     structured_surfaces: IndexMap<String, StructuredMeshSurface>,
                     wells: IndexMap<String, Well>, points: ..., polygons: ... }
impl GeoData {
    pub fn load_surface(&mut self, path) -> Result<&Surface>;     // fluent (returns ref / self)
    pub fn load_structured_surface(&mut self, path) -> Result<&StructuredMeshSurface>; // EarthVision, null-preserving
    pub fn load_well(&mut self, dir_or_files, head, kb) -> Result<&Well>;
    pub fn surface(&self, name) -> Option<&Surface>;
    pub fn well(&self, id) -> Option<&Well>;
    pub fn wells(&self) -> WellsView;            // broadcastable + filterable
}
```

Regular and structured surfaces have separate typed Rust collections so the
existing `Surface` API remains exact, but share one name-uniqueness domain and
one Python `project.surfaces` namespace. Whole-project `.pproj` persistence
stores structured entries with the existing `structured_mesh` element kind.
`model_inputs()` must fail loudly while its horizon contract cannot represent
them; it must never silently omit a structured horizon.

The Python `project.well_tops` mapping is a live, folder-aware aggregation of
actual per-bore `Top` records (unlike `project.tops`, the imported source-table
inventory). Assigning a complete `project.wells` intersection report validates
every hit and diagnostic before mutation, then atomically creates/replaces the
whole horizon and removes stale same-name picks. Outside/no-hit skips are
allowed; failures block assignment.

Provider-owned project values are separate generic `.pproj` `asset` sections,
never model/domain elements. Their collision-safe physical names live below
`@asset/<collection>/`; a versioned binary frame retains the exact canonical
JSON envelope and exact provider bytes. The envelope declares asset type,
provider, codec, and schema version. petekIO validates framing and paths but
does not interpret provider semantics, and unknown asset types/fields survive
open/save byte-for-byte. Correlation templates use `@asset/templates/<name>`;
Python exposes them as immutable, folder-aware `project.templates` snapshots
whose petekTools materialization remains lazy and optional.

`Project` implements petekTools' generic workspace provider duck:
`view_catalog()` returns an ordered metadata-only snapshot and
`view_resource(item_id, view, lane)` materializes exactly one requested role.
`project.view()` adds petekIO-native role/folder selection, surface-property
defaults, automatic metadata-only per-bore correlation discovery, optional
per-bore `ViewSpec` overrides, and stored-template resolution. Correlation
resources start hidden and gather samples only when selected. Equal-TVD picks
retain stable stratigraphic identity and represent zero-thickness intervals;
only decreasing stacks fail.
Stable IDs use canonical full paths with every segment percent-encoded; wells
add an explicit bore segment. Surface primary/attribute values stay lanes of
one item. Catalog building never calls `value_layer`, trajectory sampling, top
positioning, or log gathering. Unknown provider assets remain byte-preserved
and appear as disabled diagnostic leaves.
```python
geo = petekio.GeoData(unit="ft")
geo.load_surface("top.irap"); geo.load_well("wells/A1/", wellhead=(x,y), kb=82)
geo.wells.filter(field="Gullfaks").tops("Brent").ntg      # broadcast → Stats per well
```

---

## IO formats — phasing
- **Phase 1 (MVP):** IRAP classic (ASCII) surfaces · LAS logs (`las_rs`) ·
  deviation-survey CSV · tops CSV · scattered xyz/CSV points.
- **Phase 2:** IRAP binary (`.gri`, Fortran-record/byte-swapped — validate vs
  `xtgeo`) · ZMAP+ surfaces · GeoJSON/shapefile polygons (`geozero`) · Excel
  (`calamine`).
- **Phase 3 (3-D volumes):** SEG-Y (seismic & inversion cubes) via **`giga-segy`**
  (depend, don't reinvent) behind a `SegyReader` trait; brick-tiled storage, not
  load-all. Then OpenVDS / ZGY (no Rust → FFI, later).
- **Deferred:** GOCAD/SKUA · DLIS (bridge to `dlisio`) · RESQML · WITSML ·
  time↔depth conversion.

Every reader validates on load (geometry sane, monotonic MD, units present) and
returns a typed error, not a panic.

---

## Build phasing (for the agent)
1. `foundation` (errors, units, `GridGeometry`, geometry primitives) + `Stats`.
2. `Surface` + IRAP-classic reader + scalar/math ops + `stats`/`area_below` +
   `resample`/`sample`. (Golden tests: round-trip a known IRAP file; bilinear
   resample vs hand calc; `area_below` vs analytic.)
3. `Trajectory` (minimum-curvature) + `Well`/`Sidetrack` + `well.xyz(md)`.
   (Golden: a worked deviation survey; vertical-well degenerate case.)
4. `Log` + `Top`→interval + the `well.<top>.<log> -> Stats` ergonomic.
5. `PointSet` + `PolygonSet` (basic).
6. `GeoData` substrate + views/broadcast.
7. PyO3 bindings mirroring the above.

## Non-goals (keep it reusable)
- No reservoir concepts (wireframe, corner-point grid, volumetrics, MC) — those
  belong to consumers (e.g. SimulatoRS).
- No plotting/visualization (a separate concern; data only).
- No format completeness for its own sake — add a reader when a real need exists.

## How SimulatoRS consumes it (the point)
SimulatoRS does **no** parsing or interpolation. Its `srs-data` adapter reads a
`GeoData`: surfaces → horizons, wells' tops/logs → control points + upscaled
properties, polygons → boundary/faults — and assembles the reservoir `Wireframe`.
The library owns the entire input-data lifecycle.
