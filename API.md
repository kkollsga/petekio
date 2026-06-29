# petekIO — locked public API

> **This file is the contract.** The build must expose exactly these signatures
> (names, arguments, return types). Bodies are the implementer's; the *surface* is
> fixed. Changing a signature here requires sign-off. Rust is canonical; the
> Python (PyO3) section mirrors it. See `SPEC.md` for design/architecture.

Conventions: `Result<T> = std::result::Result<T, GeoError>`; arrays are
`ndarray` (`Array2<f64>` surfaces, `Array3<f32>` cubes); undefined = `NaN`.

---

## foundation

```rust
pub enum Unit { Feet, Metres }

pub struct Point3 { pub x: f64, pub y: f64, pub z: f64 }

pub struct BBox { pub xmin: f64, pub ymin: f64, pub xmax: f64, pub ymax: f64 }

/// Regular, rotatable areal lattice (IRAP/RMS model).
pub struct GridGeometry {
    pub xori: f64, pub yori: f64,     // origin (node 0,0)
    pub xinc: f64, pub yinc: f64,     // node spacing
    pub ncol: usize, pub nrow: usize, // node counts (i along x, j along y)
    pub rotation_deg: f64,            // CCW of the I-axis from East
    pub yflip: bool,
}
impl GridGeometry {
    pub fn node_xy(&self, i: usize, j: usize) -> (f64, f64);
    pub fn xy_to_ij(&self, x: f64, y: f64) -> Option<(f64, f64)>; // fractional node coords
    pub fn bbox(&self) -> BBox;
}

#[derive(thiserror::Error, Debug)]
pub enum GeoError { /* Io, Parse, GeometryMismatch, NotFound, OutOfRange, Unit, ... */ }
```

## Stats — the universal aggregation result

```rust
pub struct Stats {
    pub count: usize,
    pub mean: f64, pub min: f64, pub max: f64, pub std: f64, pub sum: f64,
    pub p10: f64, pub p50: f64, pub p90: f64,
}
impl Stats {
    pub fn of(values: &[f64]) -> Stats;                       // NaN-skipping
    pub fn weighted(values: &[f64], weights: &[f64]) -> Stats;
    pub fn percentile(&self, p: f64) -> f64;                  // arbitrary p in [0,1]
}
```

## Surface

```rust
pub struct Surface { pub geom: GridGeometry /* values + attributes are private */ }

impl Surface {
    // construction / IO
    pub fn new(geom: GridGeometry, values: Array2<f64>) -> Result<Surface>;
    pub fn constant(geom: GridGeometry, value: f64) -> Surface;
    pub fn load_irap_classic(path: impl AsRef<Path>) -> Result<Surface>;   // FIRST format
    pub fn save_irap_classic(&self, path: impl AsRef<Path>) -> Result<()>;

    // access
    pub fn values(&self) -> &Array2<f64>;
    pub fn sample(&self, x: f64, y: f64) -> Option<f64>;     // bilinear, NaN-aware
    pub fn attr(&self, name: &str) -> Option<&Array2<f64>>;
    pub fn set_attr(&mut self, name: &str, values: Array2<f64>) -> Result<()>;
    pub fn attr_names(&self) -> Vec<&str>;
    pub fn as_attr_surface(&self, name: &str) -> Option<Surface>;          // promote attr → ops

    // element-wise math (on the primary values; return a new Surface)
    pub fn ln(&self) -> Surface;
    pub fn log10(&self) -> Surface;
    pub fn exp(&self) -> Surface;
    pub fn sqrt(&self) -> Surface;
    pub fn abs(&self) -> Surface;
    pub fn powf(&self, n: f64) -> Surface;
    pub fn clamp_min(&self, lo: f64) -> Surface;
    pub fn clamp(&self, lo: f64, hi: f64) -> Surface;

    // surface ↔ surface (require equal geometry → GeometryMismatch otherwise)
    pub fn plus(&self, other: &Surface) -> Result<Surface>;
    pub fn minus(&self, other: &Surface) -> Result<Surface>;
    pub fn times(&self, other: &Surface) -> Result<Surface>;
    pub fn divided_by(&self, other: &Surface) -> Result<Surface>;
    pub fn thickness(top: &Surface, base: &Surface, clamp_zero: bool) -> Result<Surface>;

    // statistics / volumetrics
    pub fn stats(&self) -> Stats;
    pub fn area_below(&self, depth: f64) -> f64;             // Σ cell-area where value ≤ depth
    pub fn area_above(&self, depth: f64) -> f64;
    pub fn volume_between(&self, base: &Surface) -> Result<f64>;
    pub fn hypsometry(&self) -> Vec<(f64, f64)>;            // (depth, area) curve

    // resample (native)
    pub fn resample(&self, target: &GridGeometry) -> Surface;

    // filtering + outline
    pub fn smooth(&self, radius: usize) -> Surface;          // NaN-aware moving average; preserves the defined mask
    pub fn boundary_polygon(&self) -> Option<PolygonSet>;    // convex hull of defined nodes; None if <3

    // cube extraction (Phase 3) → a surface attribute
    pub fn slice_cube(&self, cube: &Cube, sampling: Sampling) -> Surface;
    pub fn slice_cube_window(&self, cube: &Cube, above: f64, below: f64, agg: WindowAgg) -> Surface;
}

// operator overloads (scalar + surface), mirroring the methods
impl std::ops::Add<f64> for &Surface { type Output = Surface; /* + - * / */ }
impl std::ops::Sub<&Surface> for &Surface { type Output = Result<Surface>; }
```

## Wells: `Well` → `Sidetrack` → `Trajectory` (+ tops, logs)

```rust
pub struct Well { pub id: String, pub head: (f64, f64), pub kb: f64 /* sidetracks private */ }
impl Well {
    pub fn new(id: impl Into<String>, head: (f64, f64), kb: f64) -> Well;
    pub fn sidetrack(&self, label: &str) -> Option<&Sidetrack>;
    pub fn sidetrack_mut(&mut self, label: &str) -> &mut Sidetrack;   // creates if missing
    pub fn main(&self) -> &Sidetrack;                                  // label ""
    pub fn sidetracks(&self) -> impl Iterator<Item = &Sidetrack>;
    // delegate to main/active sidetrack:
    pub fn xyz(&self, md: f64) -> Option<Point3>;
    pub fn tvd(&self, md: f64) -> Option<f64>;
    pub fn md_at_tvd(&self, tvd: f64) -> Option<f64>;
    pub fn top(&self, name: &str) -> Option<Interval>;
    pub fn log(&self, mnemonic: &str) -> Option<LogView>;
    pub fn logs(&self) -> impl Iterator<Item = &Log>;   // all main-bore logs, insertion order
    pub fn mnemonics(&self) -> Vec<&str>;
}

pub struct Sidetrack { pub label: String /* trajectories, logs, tops private */ }
impl Sidetrack {
    pub fn add_trajectory(&mut self, input: TrajectoryInput) -> Result<&mut Trajectory>; // → active
    pub fn set_active(&mut self, index: usize) -> Result<()>;
    pub fn active(&self) -> &Trajectory;
    pub fn trajectories(&self) -> &[Trajectory];
    pub fn add_log(&mut self, log: Log);
    pub fn add_tops(&mut self, tops: Vec<Top>);
    pub fn xyz(&self, md: f64) -> Option<Point3>;
    pub fn top(&self, name: &str) -> Option<Interval>;
    pub fn log(&self, mnemonic: &str) -> Option<LogView>;
    pub fn logs(&self) -> impl Iterator<Item = &Log>;   // all logs on this bore, insertion order
}

pub struct Station { pub md: f64, pub inc_deg: f64, pub azi_deg: f64 }
pub enum TrajectoryInput {
    Xyz(Vec<Point3>),
    MdIncAzi(Vec<Station>),                                  // → minimum-curvature
    Stations(Vec<Station>),
    Hold  { from: Station, to_md: f64 },
    Steer { from: Station, build_per_100: f64, turn_per_100: f64, to_md: f64 },
}
pub struct Trajectory { /* normalized positioned path */ }
impl Trajectory {
    pub fn from_input(input: TrajectoryInput, head: (f64, f64), kb: f64) -> Result<Trajectory>;  // standalone build (min-curvature)
    pub fn xyz(&self, md: f64) -> Option<Point3>;   // minimum-curvature arc interpolation between stations
    pub fn tvd(&self, md: f64) -> Option<f64>;
    pub fn md_at_tvd(&self, tvd: f64) -> Option<f64>;
    pub fn md_range(&self) -> (f64, f64);
}

pub struct Top { pub name: String, pub md: f64 }
pub struct Interval { pub name: String, pub top_md: f64, pub base_md: f64 } // base = next top / TD
impl Interval {
    pub fn log(&self, mnemonic: &str) -> Option<LogView>;    // log clipped to this interval
    pub fn thickness_md(&self) -> f64;
}

pub struct Log { pub mnemonic: String, pub unit: String /* md, values private */ }
pub struct LogView<'a> { /* a possibly interval-clipped / filtered view */ }
impl<'a> LogView<'a> {
    pub fn stats(&self) -> Stats;                            // the `well.brent.ntg` result
    pub fn stats_weighted(&self, by: &LogView) -> Stats;     // e.g. PV-weighted Sw
    pub fn filter(&self, pred: impl Fn(f64) -> bool) -> LogView<'a>;
    pub fn at_md(&self, md: f64) -> Option<f64>;
    pub fn resample(&self, step: f64) -> Log;
    pub fn values(&self) -> &[f64];
    pub fn md(&self) -> &[f64];
}
```

## PointSet & PolygonSet (simpler)

```rust
pub struct PointSet { /* N×3 coords + named attribute columns */ }
impl PointSet {
    pub fn load_csv(path: impl AsRef<Path>, x: &str, y: &str, z: &str) -> Result<PointSet>;
    pub fn load_geojson(path: impl AsRef<Path>) -> Result<PointSet>;
    pub fn load_irap_points(path: impl AsRef<Path>) -> Result<PointSet>;
    pub fn len(&self) -> usize;
    pub fn filter(&self, pred: impl Fn(Point3) -> bool) -> PointSet;
    pub fn attr(&self, name: &str) -> Option<&[f64]>;
    pub fn stats(&self, attr: &str) -> Option<Stats>;
    pub fn bbox(&self) -> BBox;
    pub fn nearest(&self, x: f64, y: f64) -> Option<usize>;
    pub fn to_surface(&self, geom: GridGeometry, method: GridMethod) -> Result<Surface>;
    pub fn regrid_min_curvature(&self, prior: &Surface) -> Result<Surface>;  // warm-started incremental re-grid on prior's lattice
}
pub enum GridMethod { Nearest, InverseDistance, MinimumCurvature }

pub struct PolygonSet { /* rings, optional Z */ }
impl PolygonSet {
    pub fn load_geojson(path: impl AsRef<Path>) -> Result<PolygonSet>;
    pub fn load_irap_polygons(path: impl AsRef<Path>) -> Result<PolygonSet>;
    pub fn load_shapefile(path: impl AsRef<Path>) -> Result<PolygonSet>;
    pub fn contains(&self, x: f64, y: f64) -> bool;
    pub fn area(&self) -> f64;
    pub fn bbox(&self) -> BBox;
    pub fn clip(&self, surface: &Surface) -> Surface;       // mask outside → NaN
}
```

## `GeoData` — the manager substrate

```rust
pub struct GeoData { pub unit: Unit /* surfaces, wells, points, polygons private */ }
impl GeoData {
    pub fn new(unit: Unit) -> GeoData;
    pub fn load_surface(&mut self, name: &str, path: impl AsRef<Path>) -> Result<&Surface>;
    pub fn load_well(&mut self, id: &str, head: (f64,f64), kb: f64,
                     files: impl AsRef<Path>) -> Result<&Well>;
    pub fn load_points(&mut self, name: &str, path: impl AsRef<Path>) -> Result<&PointSet>;
    pub fn load_polygons(&mut self, name: &str, path: impl AsRef<Path>) -> Result<&PolygonSet>;
    pub fn surface(&self, name: &str) -> Option<&Surface>;
    pub fn well(&self, id: &str) -> Option<&Well>;
    pub fn points(&self, name: &str) -> Option<&PointSet>;
    pub fn polygons(&self, name: &str) -> Option<&PolygonSet>;
    pub fn surfaces(&self) -> impl Iterator<Item = &Surface>;
    pub fn surfaces_named(&self) -> impl Iterator<Item = (&str, &Surface)>;
    pub fn polygons_named(&self) -> impl Iterator<Item = (&str, &PolygonSet)>;
    pub fn wells(&self) -> WellsView;

    /// Model-ready inputs — the consumer contract (see below). Assembles
    /// normalize→validate→interpret→characterise across the project.
    /// Surface and PolygonSet are Clone (horizons/boundary are cloned out).
    pub fn model_inputs(&self) -> Result<ModelInputs>;
}
pub struct WellsView<'a> { /* broadcastable, filterable */ }
impl<'a> WellsView<'a> {
    pub fn filter(&self, pred: impl Fn(&Well) -> bool) -> WellsView<'a>;
    pub fn iter(&self) -> impl Iterator<Item = &Well>;
    pub fn tops(&self, name: &str) -> WellsView<'a>;         // narrow to wells with that top
}
```

## Model-ready inputs — the consumer contract

> The seam petekSim (and other consumers) build to. `GeoData::model_inputs()`
> returns inputs already **normalized → validated → interpreted → uncertainty-
> characterised**, in canonical units and provenance-flagged — consumers map them
> into their domain and derive nothing. Assembly stages live in
> `analysis::{normalize, validate, model_inputs}`; GATE-0 locks this surface.

```rust
// foundation — uncertainty & provenance vocabulary
pub enum Provenance { HardData, Interpolated, Defaulted, Assumed }
pub enum Distribution {
    Deterministic,
    Uniform { lo: f64, hi: f64 },
    Triangular { lo: f64, mode: f64, hi: f64 },
    Normal { mean: f64, std: f64 },
    LogNormal { mu: f64, sigma: f64 },
}
pub struct Uncertain { pub value: f64, pub distribution: Distribution, pub provenance: Provenance }
impl Uncertain {
    pub fn hard(value: f64) -> Uncertain;       // Deterministic + HardData (measured point datum)
    pub fn defaulted(value: f64) -> Uncertain;  // Deterministic + Defaulted
    pub fn assumed(value: f64) -> Uncertain;    // Deterministic + Assumed
    pub fn uniform(lo: f64, hi: f64) -> Uncertain;            // midpoint estimate, Interpolated
    pub fn triangular(lo: f64, mode: f64, hi: f64) -> Uncertain;  // mode estimate
    pub fn normal(mean: f64, std: f64) -> Uncertain;         // mean estimate
    pub fn lognormal(mu: f64, sigma: f64) -> Uncertain;      // exp(mu) median estimate
    pub fn from_stats(stats: &Stats, provenance: Provenance) -> Uncertain;  // Normal, or Deterministic if <2 / no spread
    pub fn with_provenance(self, provenance: Provenance) -> Uncertain;
}

// foundation::Unit — areal conversion backing reservoir_area_acres
impl Unit { pub fn area_to_acres(self, area_in_unit_sq: f64) -> f64; }

// analysis — the contract (consumed by GeoData::model_inputs)
pub struct ModelInputs { pub summary: SummaryInputs, pub spatial: SpatialInputs }

pub struct SummaryInputs {              // scalars in canonical units, each Uncertain
    pub reservoir_area_acres: Uncertain,
    pub net_pay_ft: Uncertain,
    pub porosity_frac: Uncertain,
    pub water_saturation_frac: Uncertain,
    pub net_to_gross_frac: Uncertain,
    pub owc_ft: Option<Uncertain>,      // fluid contacts
    pub goc_ft: Option<Uncertain>,
}
pub struct SpatialInputs {              // for the 3D grid build + upscaling
    pub boundary: Option<PolygonSet>,
    pub horizons: Vec<HorizonInput>,    // surface.resample(&GridGeometry) onto the consumer lattice
    pub well_curves: Vec<WellCurveInput>,
}
pub struct HorizonInput { pub name: String, pub surface: Surface, pub provenance: Provenance }
pub struct WellCurveInput {
    pub well_id: String, pub mnemonic: String,  // canonical post-normalize, e.g. "PHIE"
    pub md: Vec<f64>, pub values: Vec<f64>,
    pub xyz: Vec<[f64; 3]>,                      // world (x,y,z=TVD) per sample, 1:1 with md; [NaN;3] if unpositioned
    pub provenance: Provenance,
}

// analysis::normalize — canonicalisation passes (the first half of the pipeline)
pub fn canonical_mnemonic(raw: &str) -> String;          // vendor LAS mnemonic → canonical (PHI→PHIE…); unknown passes through uppercased
pub fn parse_length_unit(s: &str) -> Option<Unit>;        // "m"/"ft"/… → Unit
pub fn is_percent_unit(s: &str) -> bool;
pub fn harmonise_fraction(value: f64, unit: &str) -> f64; // percent → fraction
pub fn harmonise_length(value: f64, from: Unit, to: Unit) -> f64;
pub struct NameMap { /* case-insensitive alias → canonical; identity for unknowns */ }
impl NameMap {
    pub fn new() -> NameMap;
    pub fn from_pairs(pairs: impl IntoIterator<Item = (String, String)>) -> NameMap;
    pub fn insert(&mut self, alias: impl Into<String>, canonical: impl Into<String>);
    pub fn canonical(&self, name: &str) -> String;
}

// analysis::validate — physical validity ranges; out-of-range → NaN (undefined)
pub fn validity_range(canonical_mnemonic: &str) -> Option<(f64, f64)>;  // inclusive (lo,hi), None = unranged
pub fn in_range(canonical_mnemonic: &str, value: f64) -> bool;           // NaN never in range
pub fn mask_out_of_range(canonical_mnemonic: &str, values: &mut [f64]) -> usize;  // → NaN in place, returns count rejected
// Provenance is assigned at derivation (measured→HardData, gridded→Interpolated, default→Defaulted), not here.

// analysis::interpret — petrophysics (petekIO OWNS net_pay). Pure array kernels; manager supplies per-sample TVD.
pub struct Cutoffs { pub phi_min: f64, pub sw_max: f64, pub vsh_max: f64 }  // Default: 0.08 / 0.5 / 0.5 (flag Assumed)
pub fn net_flags(phi: &[f64], sw: &[f64], vsh: Option<&[f64]>, cut: &Cutoffs) -> Vec<bool>;  // per-sample reservoir/pay flag
pub fn net_pay(depth: &[f64], net: &[bool]) -> f64;        // Σ representative (Voronoi) thickness over net samples; depth = TVD
pub fn net_to_gross(depth: &[f64], net: &[bool]) -> f64;   // net_pay / gross span
pub fn leverett_j(pc: f64, ift: f64, perm: f64, phi: f64) -> f64;  // (Pc/ift)·√(perm/phi), consistent units; NaN if phi≤0

// analysis::characterise — fit a Distribution + Provenance from samples (petekIO characterises, consumer samples)
pub enum DistributionShape { Normal, Triangular, LogNormal }  // Triangular = P10/P50/P90; LogNormal fitted on ln of positives
pub fn characterise(values: &[f64], shape: DistributionShape, provenance: Provenance) -> Uncertain;  // <2 defined → Deterministic
```

## Cube (Phase 3 — locked sketch)

```rust
pub enum Domain { Time, Depth }
pub enum PropertyKind { Amplitude, AcousticImpedance, VpVs, Porosity, Other(String) }
pub enum Sampling { Nearest, Trilinear }
pub enum WindowAgg { Min, Max, Rms, Mean }

pub struct CubeGeometry {
    pub areal: GridGeometry,
    pub zori: f64, pub zinc: f64, pub nz: usize,
    pub ilines: Vec<i32>, pub xlines: Vec<i32>,
    pub domain: Domain,
}
pub struct Cube { pub geom: CubeGeometry, pub property: PropertyKind, pub unit: String }
impl Cube {
    pub fn load_segy(path: impl AsRef<Path>, iline_byte: u16, xline_byte: u16) -> Result<Cube>;
    pub fn sample(&self, x: f64, y: f64, z: f64) -> Option<f32>;
    pub fn inline(&self, label: i32) -> Option<Surface>;
    pub fn zslice(&self, z: f64) -> Option<Surface>;
    pub fn stats(&self) -> Stats;
    pub fn resample(&self, target: &CubeGeometry) -> Cube;
}
```

---

## Python (PyO3) surface — the same contract, fluent

```python
import petekio

geo = petekio.GeoData(unit="ft")
geo.load_surface("top", "top.irap")
top, base = geo.surface("top"), geo.surface("base")

thick = (base - top).clamp_min(0)        # operator overloads
trend = top.attr["seismic"].ln()
top.stats.p50                            # Stats fields as attributes
top.area_below(8240)
ongrid = top.resample(grid_geom)

geo.load_well("15/9-A1", wellhead=(1200, 1500), kb=82, files="wells/A1/")
w = geo.well("15/9-A1")
w.xyz(2450)                              # interpolated position at MD

# Standalone trajectory from a directional survey (no project needed):
traj = petekio.Trajectory.from_stations(      # [(md, inc_deg, azi_deg), ...]
    [(0, 0, 145), (1200, 0, 145), (1900, 57, 145)], head=(558650, 6812460), kb=27.3)
traj.tvd(1655.81)                        # subsea TVD (RKB = + kb); xyz / md_at_tvd / md_range too
w.brent.ntg                             # -> Stats  (dynamic: __getattr__ top → log)
w.brent.phie.mean
geo.wells.filter(field="Gullfaks").tops("Brent")   # broadcast view
```

Python rules: `Stats` fields exposed as read-only attributes; operators (`+ - * /`)
on `Surface`; `surface.attr["name"]` indexed access; `well.<top>.<log>` resolves
via `__getattr__` (top interval → log → `Stats`). Bindings are thin: every method
delegates to the Rust API above.
