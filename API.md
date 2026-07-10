# petekIO — locked public API

> **This file is the contract.** The build must expose exactly these signatures
> (names, arguments, return types). Bodies are the implementer's; the *surface* is
> fixed. Changing a signature here requires sign-off. Rust is canonical; the
> Python (PyO3) section mirrors it. See `SPEC.md` for design/architecture.

Conventions: `Result<T> = std::result::Result<T, GeoError>`; arrays are
`ndarray` (`Array2<f64>` surfaces, `Array3<f32>` cubes); undefined = `NaN`.

---

## Format Detection

```rust
pub enum FormatKind {
    Cps3Grid,
    Cps3Lines,
    IrapClassicGrid,
    IrapClassicPoints,
    EarthVisionGrid,
    Las,
    WellPath,
    PetrelTops,
    CrsMetaXml,
    GeoJson,
    CsvPoints,
    Unknown,
}

/// Bounded content-sniffing detector. Reads only leading header bytes; file
/// extension is a fallback/tiebreaker, not the primary authority.
pub fn detect(path: impl AsRef<Path>) -> Result<FormatKind>;
```

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
    // shell lifts (free, lossless — see "Geometry shells"; impls live in core)
    pub fn to_structured_shell(&self) -> StructuredShell;      // per-node XY computed; nominal = self
    pub fn to_mesh_shell(&self) -> Result<MeshShell>;          // quad-split, consistent diagonal, CCW; labels (0,i,j)
}

#[derive(thiserror::Error, Debug)]
pub enum GeoError { /* Io, Parse, Format, GeometryMismatch, GeometryInference, NotFound, OutOfRange, Unsupported, Unit, ... */ }
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
    pub fn geomean(values: &[f64]) -> f64;                    // geometric mean of positives (e.g. permeability)
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
    pub fn load_cps3_grid(path: impl AsRef<Path>) -> Result<Surface>;      // CPS-3 regular grid (row-major, south→north; origin at ymin, matches IRAP)
    pub fn save_irap_classic(&self, path: impl AsRef<Path>) -> Result<()>;

    // access
    pub fn values(&self) -> &Array2<f64>;
    pub fn sample(&self, x: f64, y: f64) -> Option<f64>;     // bilinear (petektools kernel); None outside grid or if NEAREST corner NaN; else renormalized over finite corners. Exact under rotation.
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

    // resample (petektools kernel, Bilinear; one-home). Kernel NaN-corner policy
    // (see sample). Err(Unsupported) on a ROTATED source/target (axis-aligned kernel
    // only; yflip OK) pending suite grid-rotation. Was `-> Surface` (infallible)
    // pre-0.2.9.
    pub fn resample(&self, target: &GridGeometry) -> Result<Surface>;

    // filtering + outline
    pub fn smooth(&self, radius: usize) -> Surface;          // NaN-aware moving average; preserves the defined mask
    pub fn edge(&self) -> Option<PolygonSet>;                // convex hull of defined nodes; None if <3

    // shell lifts (free, lossless; ALL attribute lanes carried 1:1, node identity preserved)
    pub fn to_structured_mesh(&self) -> Result<StructuredMeshSurface>;
    pub fn to_tri_surface(&self) -> Result<TriSurface>;      // grid quad-split (consistent diagonal)

    // iso-lines + value layer (all three surface levels expose these — see "Geometry shells")
    pub fn iso_lines(&self, interval: Option<f64>, levels: Option<Vec<f64>>, attr: Option<&str>)
        -> Result<Vec<(f64, Vec<Vec<[f64; 2]>>)>>;           // NaN-aware marching triangles; explicit levels win; interval → levels aligned to its multiples over the value range; deterministic chaining
    pub fn value_layer(&self, attr: Option<&str>) -> Result<ValueLayer>;  // the viewer trimesh bundle

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
pub struct Well { pub id: String, pub head: (f64, f64), pub kb: f64 /* crs + sidetracks private */ }
impl Well {
    pub fn new(id: impl Into<String>, head: (f64, f64), kb: f64) -> Well;
    pub fn crs(&self) -> Option<&str>;                  // CRS label (provenance; never reprojected)
    pub fn set_crs(&mut self, crs: impl Into<String>);
    pub fn sidetrack(&self, label: &str) -> Option<&Sidetrack>;
    pub fn sidetrack_mut(&mut self, label: &str) -> &mut Sidetrack;   // creates if missing
    pub fn main(&self) -> &Sidetrack;                                  // label ""
    pub fn sidetracks(&self) -> impl Iterator<Item = &Sidetrack>;
    pub fn bores(&self) -> impl Iterator<Item = &str>;                 // bore labels, main "" first
    pub fn bore_id(&self, label: &str) -> String;      // "<id> <bore>" (id for main) — the model_inputs key
    // Multi-bore = >1 bore carries a trajectory. Then the delegating accessors
    // below need an explicit bore: enumerate bores() + work per-bore via
    // sidetrack(), or set a default. NEVER silently pick one (silent-empty bug).
    pub fn is_multibore(&self) -> bool;
    pub fn set_default_bore(&mut self, label: &str) -> Result<()>;     // Err if no such bore
    pub fn default_bore(&self) -> Option<&str>;
    pub fn clear_default_bore(&mut self);
    // Delegate to the *resolved* bore, in order: (1) the **default bore** if set;
    // (2) the **single-trajectory rule** — the sole trajectory-bearing bore when
    // exactly one bore has a trajectory (a lone deviated sidetrack positions its
    // logs/tops through it regardless of label); (3) the main bore. A multi-bore
    // well with no default resolves through the (empty) main → None; select a bore.
    pub fn xyz(&self, md: f64) -> Option<Point3>;   // z = negative-down elevation (matches Surface z; = -tvd)
    pub fn tvd(&self, md: f64) -> Option<f64>;       // positive-down TVDSS
    pub fn md_at_tvd(&self, tvd: f64) -> Option<f64>;
    pub fn top(&self, name: &str) -> Option<Interval>;
    pub fn log(&self, mnemonic: &str) -> Option<LogView>;
    pub fn logs(&self) -> impl Iterator<Item = &Log>;   // all resolved-bore logs, insertion order
    pub fn mnemonics(&self) -> Vec<&str>;
    pub fn zones(&self) -> Vec<Interval>;            // formation zones, in strat order if set, else MD
    pub fn zone_stats(&self, mnemonic: &str) -> Vec<(String, Stats)>;  // per-zone average(mean)+sum
    pub fn contacts(&self) -> impl Iterator<Item = &FluidContact>;     // non-strat picks on the resolved bore
    pub fn contact(&self, name: &str) -> Option<&FluidContact>;
    pub fn set_strat_order(&mut self, order: &[String]);  // push lithostrat column into every bore
}

pub struct Sidetrack { pub label: String /* trajectories, logs, tops private */ }
impl Sidetrack {
    pub fn add_trajectory(&mut self, input: TrajectoryInput) -> Result<&mut Trajectory>; // → active
    pub fn set_active(&mut self, index: usize) -> Result<()>;
    pub fn active(&self) -> &Trajectory;
    pub fn trajectories(&self) -> &[Trajectory];
    pub fn add_log(&mut self, log: Log);
    pub fn add_tops(&mut self, tops: Vec<Top>);
    pub fn set_strat_order(&mut self, order: &[String]);  // strat order; coincident-MD tie interval → deepest member
    // Per-bore access is first-class: everything a consumer needs to treat a bore
    // as an independent positioned "well" (used per-bore for a multi-sidetrack well).
    pub fn xyz(&self, md: f64) -> Option<Point3>;
    pub fn tvd(&self, md: f64) -> Option<f64>;
    pub fn md_at_tvd(&self, tvd: f64) -> Option<f64>;
    pub fn top(&self, name: &str) -> Option<Interval>;
    pub fn log(&self, mnemonic: &str) -> Option<LogView>;
    pub fn logs(&self) -> impl Iterator<Item = &Log>;   // all logs on this bore, insertion order
    pub fn mnemonics(&self) -> Vec<&str>;               // curve mnemonics on this bore, insertion order
    pub fn zones(&self) -> Vec<Interval>;               // strat order when set (see set_strat_order), else MD
    pub fn zone_stats(&self, mnemonic: &str) -> Vec<(String, Stats)>;
    pub fn contacts(&self) -> impl Iterator<Item = &FluidContact>;
    pub fn contact(&self, name: &str) -> Option<&FluidContact>;
}

pub struct Station { pub md: f64, pub inc_deg: f64, pub azi_deg: f64 }
pub enum TrajectoryInput {
    Xyz(Vec<Point3>),
    MdIncAzi(Vec<Station>),                                  // → minimum-curvature
    Stations(Vec<Station>),
    PositionedSurvey(Vec<(Station, Point3)>),               // explicit positions; preserves MD (e.g. .wellpath)
    Hold  { from: Station, to_md: f64 },
    Steer { from: Station, build_per_100: f64, turn_per_100: f64, to_md: f64 },
}
pub struct Trajectory { /* normalized positioned path */ }
impl Trajectory {
    pub fn from_input(input: TrajectoryInput, head: (f64, f64), kb: f64) -> Result<Trajectory>;  // standalone build (min-curvature)
    pub fn xyz(&self, md: f64) -> Option<Point3>;   // min-curvature arc interp; z = negative-down elevation (= -tvd)
    pub fn tvd(&self, md: f64) -> Option<f64>;
    pub fn md_at_tvd(&self, tvd: f64) -> Option<f64>;
    pub fn md_range(&self) -> (f64, f64);
}

pub struct Top { pub name: String, pub md: f64 }
pub struct FluidContact { pub name: String, pub md: f64 } // OWC/GOC/FWL etc.; not a zone top
pub struct Interval { pub name: String, pub top_md: f64, pub base_md: f64 } // base = next top / TD
impl Interval {
    pub fn log(&self, mnemonic: &str) -> Option<LogView>;    // log clipped to this interval
    pub fn thickness_md(&self) -> f64;
}

pub enum LogKind { Log, Core }                            // continuous log vs core-derived
pub struct Log { pub mnemonic: String, pub unit: String /* kind, md, values private */ }
impl Log {
    pub fn kind(&self) -> LogKind;
    pub fn with_kind(self, kind: LogKind) -> Log;
    pub fn history(&self) -> &[String];
}
pub struct LogView<'a> { /* a possibly interval-clipped / filtered view */ }
impl<'a> LogView<'a> {
    pub fn stats(&self) -> Stats;                            // the `well.brent.ntg` result
    pub fn stats_weighted(&self, by: &LogView) -> Stats;     // e.g. PV-weighted Sw
    pub fn filter(&self, pred: impl Fn(f64) -> bool) -> LogView<'a>;
    pub fn at_md(&self, md: f64) -> Option<f64>;
    pub fn resample(&self, step: f64) -> Log;                // onto a regular grid of spacing `step`
    pub fn resample_onto(&self, targets: &[f64]) -> Vec<f64>; // onto arbitrary ASCENDING targets (O(n+k) merge-walk; NaN out-of-span)
    pub fn values(&self) -> &[f64];
    pub fn history(&self) -> &[String];
    pub fn md(&self) -> &[f64];
}
```

## PointSet & PolygonSet (simpler)

```rust
pub struct PointSet { /* N×3 coords + named attribute columns */ }
pub enum GeometryEdge { Occupied, ConvexHull, FullRect }
impl PointSet {
    pub fn from_coords(coords: Vec<[f64; 3]>) -> PointSet;    // in-memory, no file
    pub fn z_stats(&self) -> Stats;                          // stats over the z coordinate
    pub fn load_csv(path: impl AsRef<Path>, x: &str, y: &str, z: &str) -> Result<PointSet>;
    pub fn load_geojson(path: impl AsRef<Path>) -> Result<PointSet>;
    pub fn load_irap_points(path: impl AsRef<Path>) -> Result<PointSet>;   // plain X Y Z; header-sniffed (foreign → GeoError::Format)
    pub fn load_earthvision_grid(path: impl AsRef<Path>) -> Result<PointSet>;  // EarthVision grid ASCII (null nodes dropped)
    pub fn len(&self) -> usize;
    pub fn coords(&self) -> &[[f64; 3]];                     // raw [x,y,z] points, load order (read side of from_coords)
    pub fn filter(&self, pred: impl Fn(Point3) -> bool) -> PointSet;
    pub fn attr(&self, name: &str) -> Option<&[f64]>;
    pub fn stats(&self, attr: &str) -> Option<Stats>;
    pub fn bbox(&self) -> BBox;
    pub fn nearest(&self, x: f64, y: f64) -> Option<usize>;
    pub fn infer_geometry(&self, tolerance: f64) -> Result<GridGeometry>;
    pub fn infer_geometry_with_edge(&self, tolerance: f64, edge: GeometryEdge) -> Result<(GridGeometry, PolygonSet)>;
    pub fn to_surface(&self, geom: GridGeometry, method: GridMethod) -> Result<Surface>;
    pub fn to_structured_surface(&self, tolerance: f64, edge: GeometryEdge) -> Result<StructuredMeshSurface>;
    pub fn detect_topology(&self, nominal_cell: Option<f64>) -> Result<(Option<PointSet>, TopologyReport)>;
    pub fn to_tri_surface(&self, max_link: Option<f64>, max_bridge: Option<f64>) -> Result<TriSurface>;   // both in CELLS; max_link in (sqrt2, 2), None = 1.8; max_bridge >= max_link admits open-seam edges (fringe/fault/gap), None = strictly lattice-closed
    pub fn regrid_min_curvature(&self, prior: &Surface) -> Result<Surface>;  // warm-started incremental re-grid on prior's lattice
}
pub enum GridMethod { Nearest, InverseDistance, MinimumCurvature }

/// The outcome of recovering `(column, row)` from unlabelled surface points.
/// `verified()` is the gate: `detect_topology` returns labelled points only when it
/// holds. An unverified report means the surface is fault-cut — represent it as a
/// triangulated network, not a structured mesh. Spec: `surface_topology_walk_spec`.
pub struct TopologyReport {
    pub detected_cell_i: f64,          // the two increments are resolved separately
    pub detected_cell_j: f64,
    pub detected_azimuth_deg: f64,
    pub distinct_nodes: usize,
    pub assigned: usize,
    pub conflicts: usize,
    pub coincident_dropped: usize,     // same XY and same Z: harmless
    pub coincident_ambiguous: usize,   // same XY, different Z: unresolvable
    pub stalled_frontier: usize,       // the fault traces, in point-index form
    pub blocks: usize,                 // fault blocks; verified() requires exactly 1
    pub largest_block: usize,
}
impl TopologyReport { pub fn verified(&self) -> bool; }
```

## Geometry shells — the three-level system

> Geometry is a **flat empty shell**: purely topological/positional, never a
> function of z. Three levels of increasing complexity — level 1 is the rigid
> `GridGeometry` (8 scalars, XY computed); levels 2 and 3 are below. A surface
> is *shell + property lanes*; shells are immutable once built and shared via
> `Arc`, so N properties/clones never repeat geometry in memory. Conversions
> go **up for free** (lossless, node identity preserved) and **down by
> inference** (`infer_grid` fits a regular lattice or errors). Derived walk
> indexes (the corner table) are never persisted.

```rust
/// Level 2: (i, j)-organized nodes with explicit per-node XY.
pub struct StructuredShell { /* ncol, nrow, x, y, nominal_geometry, edge */ }
impl StructuredShell {
    pub fn new(x: Array2<f64>, y: Array2<f64>,
               nominal_geometry: Option<GridGeometry>, edge: PolygonSet) -> Result<StructuredShell>;
    pub fn ncol(&self) -> usize;
    pub fn nrow(&self) -> usize;
    pub fn x(&self) -> &Array2<f64>;                          // shape (ncol, nrow)
    pub fn y(&self) -> &Array2<f64>;
    pub fn nominal_geometry(&self) -> Option<&GridGeometry>;  // metadata only
    pub fn edge(&self) -> &PolygonSet;
    pub fn node_xy(&self, i: usize, j: usize) -> Result<(f64, f64)>;
    pub fn bbox(&self) -> BBox;
    pub fn to_mesh_shell(&self) -> Result<MeshShell>;         // quad-split (consistent diagonal, CCW); labels (0,i,j)
    pub fn infer_grid(&self, tolerance: f64) -> Result<GridGeometry>;  // fit; Err when curvilinear
}

/// A node's place in the walked grid: fault block + (column, row) inside it.
pub type WalkLabel = (u32, i32, i32);

/// Level 3: integer node ids with explicit 2-D XY + triangle topology.
pub struct MeshShell { /* nodes, triangles, wireframe, edge, labels; lazy corner table */ }
impl MeshShell {
    pub fn new(nodes: Vec<[f64; 2]>, triangles: Vec<[u32; 3]>, wireframe: Vec<[u32; 2]>,
               edge: PolygonSet, labels: Vec<Option<WalkLabel>>) -> Result<MeshShell>;
        // validates node refs + labels len + EDGE-MANIFOLD (no undirected edge in >2 triangles)
    pub fn nodes(&self) -> &[[f64; 2]];                        // 2-D by design — never a function of z
    pub fn triangles(&self) -> &[[u32; 3]];                    // CCW
    pub fn wireframe_edges(&self) -> Vec<[u32; 2]>;            // quad-dominant (interior cell diagonals hidden)
    pub fn labels(&self) -> &[Option<WalkLabel>];              // per node; kept on the shell
    pub fn edge(&self) -> &PolygonSet;
    pub fn n_nodes(&self) -> usize;
    pub fn n_triangles(&self) -> usize;
    pub fn components(&self) -> usize;
    pub fn bbox(&self) -> BBox;
    pub fn corner_table(&self) -> &CornerTable;                // lazy (OnceLock); derived, NEVER serialized
    pub fn infer_grid(&self, tolerance: f64) -> Result<GridGeometry>;  // labels-exact fit when single-block, else coordinate detection; Err when not regular
}

/// The derived walkability index: per corner (3t+k) the opposite corner in the
/// adjacent triangle, plus a representative corner per vertex.
pub const NO_CORNER: u32 = u32::MAX;   // boundary / unused sentinel (shell::corner)
pub struct CornerTable { /* opposite, vertex_corner */ }
impl CornerTable {
    pub fn opposite(&self, corner: u32) -> u32;                // NO_CORNER on the boundary
    pub fn vertex_corner(&self, vertex: u32) -> u32;
    pub fn n_corners(&self) -> usize;                          // 3 * n_triangles
    pub fn triangle_of(corner: u32) -> usize;
}

/// A property lane presented on a trimesh — the bundle the petektools viewer
/// consumes (`ValueLayer::KIND == "trimesh"`; primary lane name = `ValueLayer::PRIMARY == "values"`).
pub struct ValueLayer {
    pub name: String,           // attr name, or "values" for the primary lane
    pub nodes: Vec<[f64; 2]>,
    pub triangles: Vec<[u32; 3]>,
    pub values: Vec<f64>,       // per node; NaN allowed
    pub range: [f64; 2],        // finite min/max ([NaN, NaN] when nothing finite)
}
```

```rust
/// The triangulated fallback for a fault-cut surface: the original points, unmoved,
/// as one connected sheet. A level-3 surface: an `Arc`-shared `MeshShell` (geometry)
/// + a primary per-node z lane + named attribute lanes. Spec: `surface_tin_fallback_spec`.
pub const DEFAULT_MAX_LINK: f64 = 1.8;   // cells
pub struct TriSurface { /* Arc<MeshShell> + values + attributes */ }
impl TriSurface {
    pub fn from_shell(shell: Arc<MeshShell>, values: Vec<f64>) -> Result<TriSurface>;
    pub fn kind(&self) -> &'static str;
    pub fn shell(&self) -> &Arc<MeshShell>;     // the geometry, shared — never copied per lane
    pub fn points(&self) -> Vec<[f64; 3]>;      // shell XY zipped with z — the input points, unmoved (was &[[f64;3]] pre-shell)
    pub fn values(&self) -> &[f64];             // primary per-node lane (z); NaN = undefined
    pub fn triangles(&self) -> &[[u32; 3]];     // CCW, indices into points()
    pub fn wireframe_edges(&self) -> Vec<[u32; 2]>; // unique edges minus interior cell diagonals — the geometry's flat-shell wireframe (purely topological, never a function of z)
    pub fn edge(&self) -> &PolygonSet;
    pub fn components(&self) -> usize;          // >1 means the mesh honours a fault
    pub fn to_points(&self) -> PointSet;
    pub fn bbox(&self) -> BBox;
    pub fn stats(&self) -> Stats;
    // attribute lanes (mirror Surface's; one value per shell node)
    pub fn attr(&self, name: &str) -> Option<&[f64]>;
    pub fn set_attr(&mut self, name: &str, values: Vec<f64>) -> Result<()>;
    pub fn attr_names(&self) -> Vec<&str>;
    pub fn as_attr_surface(&self, name: &str) -> Option<TriSurface>;   // promote lane → primary, SAME shell
    // conversions (down = lossy) + iso/value-layer (same signatures as Surface)
    pub fn infer_grid(&self, tolerance: f64) -> Result<GridGeometry>;  // fit; Err when not regular
    pub fn resample(&self, target: &GridGeometry, method: GridMethod) -> Result<Surface>;  // grids primary + ALL attr lanes
    pub fn iso_lines(&self, interval: Option<f64>, levels: Option<Vec<f64>>, attr: Option<&str>)
        -> Result<Vec<(f64, Vec<Vec<[f64; 2]>>)>>;
    pub fn value_layer(&self, attr: Option<&str>) -> Result<ValueLayer>;
    pub fn save(&self, path: impl AsRef<Path>) -> Result<()>;          // one-section .pproj; shell stored ONCE + N lanes
    pub fn load(path: impl AsRef<Path>) -> Result<TriSurface>;
}

/// A `(column, row)`-indexed surface carrying explicit per-node XY: the exact home for
/// Petrel/EarthVision exports whose nodes are fault-shifted or curvilinear and therefore
/// lie on no single `GridGeometry`. A level-2 surface: an `Arc`-shared `StructuredShell`
/// (geometry) + a primary value lane + named attribute lanes. `nominal_geometry` is
/// metadata, never the coordinates.
pub struct StructuredMeshSurface { /* Arc<StructuredShell> + values + attributes */ }
impl StructuredMeshSurface {
    pub fn new(x: Array2<f64>, y: Array2<f64>, values: Array2<f64>,
               nominal_geometry: Option<GridGeometry>, edge: PolygonSet) -> Result<Self>;
    pub fn from_shell(shell: Arc<StructuredShell>, values: Array2<f64>) -> Result<Self>;
    pub fn kind(&self) -> &'static str;
    pub fn shell(&self) -> &Arc<StructuredShell>;  // the geometry, shared — never copied per lane
    pub fn ncol(&self) -> usize;
    pub fn nrow(&self) -> usize;
    pub fn x(&self) -> &Array2<f64>;
    pub fn y(&self) -> &Array2<f64>;
    pub fn values(&self) -> &Array2<f64>;
    pub fn nominal_geometry(&self) -> Option<&GridGeometry>;  // metadata only
    pub fn edge(&self) -> &PolygonSet;
    pub fn node_xy(&self, i: usize, j: usize) -> Result<(f64, f64)>;
    pub fn z(&self, i: usize, j: usize) -> Result<f64>;
    pub fn to_points(&self) -> PointSet;   // exact inverse of PointSet::to_structured_surface
    pub fn bbox(&self) -> BBox;
    pub fn stats(&self) -> Stats;
    // attribute lanes (mirror Surface's; each shaped (ncol, nrow))
    pub fn attr(&self, name: &str) -> Option<&Array2<f64>>;
    pub fn set_attr(&mut self, name: &str, values: Array2<f64>) -> Result<()>;
    pub fn attr_names(&self) -> Vec<&str>;
    pub fn as_attr_surface(&self, name: &str) -> Option<StructuredMeshSurface>;  // promote lane → primary, SAME shell
    // conversions (up = free/lossless carrying all lanes; down = fit/resample)
    pub fn to_tri_surface(&self) -> Result<TriSurface>;               // quad-split; node identity on labels
    pub fn infer_grid(&self, tolerance: f64) -> Result<GridGeometry>; // fit; Err when curvilinear
    pub fn resample(&self, target: &GridGeometry, method: GridMethod) -> Result<Surface>;  // grids primary + ALL attr lanes
    pub fn iso_lines(&self, interval: Option<f64>, levels: Option<Vec<f64>>, attr: Option<&str>)
        -> Result<Vec<(f64, Vec<Vec<[f64; 2]>>)>>;
    pub fn value_layer(&self, attr: Option<&str>) -> Result<ValueLayer>;
    pub fn save(&self, path: impl AsRef<Path>) -> Result<()>;         // one-section .pproj; shell stored ONCE + N lanes
    pub fn load(path: impl AsRef<Path>) -> Result<StructuredMeshSurface>;
}

pub struct PolygonSet { /* rings, optional Z */ }
impl PolygonSet {
    pub fn from_rings(rings: Vec<Vec<[f64; 3]>>) -> PolygonSet;   // in-memory, no file (Z ignored)
    pub fn load_geojson(path: impl AsRef<Path>) -> Result<PolygonSet>;
    pub fn load_irap_polygons(path: impl AsRef<Path>) -> Result<PolygonSet>;
    pub fn load_shapefile(path: impl AsRef<Path>) -> Result<PolygonSet>;
    pub fn load_cps3_lines(path: impl AsRef<Path>) -> Result<PolygonSet>;  // CPS-3 polyline blocks (`->`-separated)
    pub fn contains(&self, x: f64, y: f64) -> bool;
    pub fn area(&self) -> f64;
    pub fn bbox(&self) -> BBox;
    pub fn rings(&self) -> Vec<Vec<[f64; 3]>>;              // exterior outline vertices per polygon (z=0, closed)
    pub fn clip(&self, surface: &Surface) -> Surface;       // mask outside → NaN
}
```

## `GeoData` — the manager substrate

```rust
pub struct GeoData { pub unit: Unit /* surfaces, wells, points, polygons private */ }
impl GeoData {
    pub fn new(unit: Unit) -> GeoData;
    pub fn load_surface(&mut self, name: &str, path: impl AsRef<Path>) -> Result<&Surface>; // content-first detect(); Unknown falls back to extension
    pub fn load_well(&mut self, id: &str, head: (f64,f64), kb: f64,
                     files: impl AsRef<Path>) -> Result<&Well>;  // content-first tree walk: one .wellpath→main bore (co-locates logs/tops), many→one bore each; LAS→logs; .csv→tops; crsmeta.xml→Well::crs label
    pub fn set_curve_aliases(&mut self, aliases: NameMap);  // opt-in STICKY: canonicalize log mnemonics at load (map→table→vintage strip); off by default (raw preserved)
    pub fn load_well_with(&mut self, id: &str, head: (f64,f64), kb: f64, files: impl AsRef<Path>, aliases: Option<&NameMap>) -> Result<()>;  // per-call, NON-sticky aliases (project state preserved+restored); the IngestSpec seam
    pub fn load_well_tops(&mut self, path: impl AsRef<Path>) -> Result<usize>;  // Petrel multi-well tops → matching well+bore (well-name match folds /,-,space,case → variant-tolerant); derives strat_order across the file
    pub fn strat_order(&self) -> &[String];   // global lithostrat column from the last load_well_tops
    pub fn add_strat_hint(&mut self, above: &str, below: &str);  // soft hint; fills stalemates, data wins
    pub fn strat_hint(&mut self, spec: &str) -> Result<()>;      // shorthand: "A < B" (A above) / "A > B"
    pub fn add_strat_hints(&mut self, hints: &StratHints);       // apply a declarative StratHints value (the IngestSpec seam)
    pub fn load_points(&mut self, name: &str, path: impl AsRef<Path>) -> Result<&PointSet>;     // content-first detect(); Unknown falls back to extension
    pub fn load_polygons(&mut self, name: &str, path: impl AsRef<Path>) -> Result<&PolygonSet>; // content-first detect(); Unknown falls back to extension
    pub fn surface(&self, name: &str) -> Option<&Surface>;
    pub fn well(&self, id: &str) -> Option<&Well>;
    pub fn well_mut(&mut self, id: &str) -> Option<&mut Well>;   // in-place e.g. Well::set_default_bore
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

    // Persistence — a single structured .pproj file (see the persistence design).
    pub fn save(&self, path: impl AsRef<Path>) -> Result<()>;          // atomic whole-project write
    pub fn open(path: impl AsRef<Path>) -> Result<GeoData>;            // materialize; model/*+unknown kinds skipped
    pub fn inspect(path: impl AsRef<Path>) -> Result<ProjectInfo>;     // manifest only (list without loading)
    pub fn split(src: impl AsRef<Path>, dst: impl AsRef<Path>, names: &[&str]) -> Result<()>; // byte-lossless
    pub fn export(src: impl AsRef<Path>, dst: impl AsRef<Path>, tags: &[&str]) -> Result<()>; // tag-filtered subset
    pub fn merge(a: impl AsRef<Path>, b: impl AsRef<Path>, dst: impl AsRef<Path>) -> Result<()>;
    pub fn owner(&self) -> Option<&str>;
    pub fn set_owner(&mut self, owner: impl Into<String>);
    pub fn tags(&self) -> &[String];
    pub fn set_tags(&mut self, tags: Vec<String>);
    pub fn set_element_tags(&mut self, name: impl Into<String>, tags: Vec<String>);
    // petekSim's opaque model sidecar (bytes petekIO never parses; per-section version):
    pub fn put_model_section(&mut self, name: impl Into<String>, tags: Vec<String>, version: u32, bytes: Vec<u8>);
    pub fn model_section_names(&self) -> Vec<String>;
    pub fn model_section(&self, name: &str) -> Option<(u32, Vec<u8>)>;
}
// Per element: Surface/Well/PointSet/PolygonSet/StructuredMeshSurface/TriSurface each
// expose `save(path)`/`load(path)` (a standalone one-section .pproj). Level-2/3 surface
// sections (kinds "structured_mesh"/"tri_surface") store the shell ONCE with N property
// lanes referencing it; derived walk indexes are never persisted; older .pproj files
// (which predate these kinds) load unchanged. Human-readable export: PointSet::export_geojson/
// export_csv, PolygonSet::export_geojson, Surface::save_irap_classic.
// ProjectInfo { owner, tags, created, modified, unit, elements: Vec<(kind, name)> }.
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

// foundation::Unit — areal conversion (project unit² → m²) backing area_m2
impl Unit { pub fn area_to_m2(self, area_in_unit_sq: f64) -> f64; }

// analysis — the contract (consumed by GeoData::model_inputs)
pub struct ModelInputs { pub summary: SummaryInputs, pub spatial: SpatialInputs }

pub struct SummaryInputs {              // scalars in base-SI units, each Uncertain
    pub area_m2: Uncertain,             // reservoir/drainage area, m²
    pub net_pay_m: Uncertain,           // net pay thickness, m
    pub porosity_frac: Uncertain,
    pub water_saturation_frac: Uncertain,
    pub net_to_gross_frac: Uncertain,
    pub owc_depth_m: Option<Uncertain>, // fluid contacts, positive-down depth in m
    pub goc_depth_m: Option<Uncertain>, // (petekStatic Contact.depth_m datum; GOC shallower than OWC)
}
pub struct SpatialInputs {              // for the 3D grid build + upscaling
    pub boundary: Option<PolygonSet>,
    pub horizons: Vec<HorizonInput>,    // surface.resample(&GridGeometry)? onto the consumer lattice (Result; Err(Unsupported) if the horizon geometry is rotated)
    pub well_curves: Vec<WellCurveInput>,  // ONE positioned curve-set PER BORE: a multi-sidetrack well emits each bore separately (well_id = "<id> <bore>")
}
pub struct HorizonInput { pub name: String, pub surface: Surface, pub provenance: Provenance }
pub struct WellCurveInput {
    pub well_id: String, pub mnemonic: String,  // bore-qualified id ("99/9-1 A"; id alone for the main bore); canonical mnemonic e.g. "PHIE"
    pub md: Vec<f64>, pub values: Vec<f64>,
    pub xyz: Vec<[f64; 3]>,                      // world (x,y,z=NEGATIVE-DOWN ELEVATION, matches Surface z) per sample, 1:1 with md; [NaN;3] if unpositioned (since 0.3.0; was TVDSS)
    pub provenance: Provenance,
}

// analysis::normalize — canonicalisation passes (the first half of the pipeline)
pub fn canonical_mnemonic(raw: &str) -> String;          // vendor LAS mnemonic → canonical (PHI→PHIE, PHIE_2025→PHIE…); SW≠SWT; unknown passes through (vintage-stripped)
pub fn canonical_mnemonic_with(raw: &str, aliases: &NameMap) -> String;  // user alias map first (resolves NTG_PhieLam vs NTG_VShale), then the table
pub fn parse_length_unit(s: &str) -> Option<Unit>;        // "m"/"ft"/… → Unit
pub fn is_percent_unit(s: &str) -> bool;
pub fn harmonise_fraction(value: f64, unit: &str) -> f64; // percent → fraction
pub fn harmonise_length(value: f64, from: Unit, to: Unit) -> f64;
pub struct NameMap { /* case-insensitive alias → canonical; identity for unknowns */ }  // Serialize/Deserialize + PartialEq + Display (sorted "alias -> canonical" rows) — a value type
impl NameMap {
    pub fn new() -> NameMap;
    pub fn from_pairs(pairs: impl IntoIterator<Item = (String, String)>) -> NameMap;
    pub fn insert(&mut self, alias: impl Into<String>, canonical: impl Into<String>);
    pub fn canonical(&self, name: &str) -> String;        // identity if unmapped
    pub fn get(&self, name: &str) -> Option<String>;      // None if unmapped (no identity fallback)
    pub fn pairs(&self) -> Vec<(String, String)>;         // sorted (alias, canonical) pairs (aliases lowercased)
    pub fn is_empty(&self) -> bool;
}

// Soft lithostratigraphic ordering hints, applied at well-tops load (names resolved at apply, loud error naming a bad token). The value behind the Python IngestSpec's strat_hints.
pub struct StratHints { /* (above, below) token pairs */ }  // Serialize/Deserialize + PartialEq + Display ("A < B" rows) — a value type
impl StratHints {
    pub fn new() -> StratHints;
    pub fn from_pairs(pairs: impl IntoIterator<Item = (String, String)>) -> StratHints;
    pub fn push(&mut self, above: impl Into<String>, below: impl Into<String>);
    pub fn push_spec(&mut self, spec: &str) -> Result<()>;  // "A < B" / "A > B" shorthand
    pub fn pairs(&self) -> &[(String, String)];
    pub fn is_empty(&self) -> bool;
}

// analysis::validate — physical validity ranges; out-of-range → NaN (undefined)
pub fn validity_range(canonical_mnemonic: &str) -> Option<(f64, f64)>;  // inclusive (lo,hi), None = unranged
pub fn in_range(canonical_mnemonic: &str, value: f64) -> bool;           // NaN never in range
pub fn mask_out_of_range(canonical_mnemonic: &str, values: &mut [f64]) -> usize;  // → NaN in place, returns count rejected
// Provenance is assigned at derivation (measured→HardData, gridded→Interpolated, default→Defaulted), not here.

// analysis::interpret — petrophysics (petekIO OWNS net_pay). Pure array kernels; manager supplies per-sample TVD.
pub struct Cutoffs { pub phi_min: f64, pub sw_max: f64, pub vsh_max: f64 }  // Default: 0.08 / 0.5 / 0.5 (flag Assumed). Serialize/Deserialize + PartialEq + Display ("phi>=0.080  Sw<=0.500  Vsh<=0.500") — the core value behind the Python NetSettings spec.
pub fn net_flags(phi: &[f64], sw: &[f64], vsh: Option<&[f64]>, cut: &Cutoffs) -> Vec<bool>;  // per-sample reservoir/pay flag
pub fn net_pay(depth: &[f64], net: &[bool]) -> f64;        // Σ representative (Voronoi) thickness over net samples; depth = TVD
pub fn net_to_gross(depth: &[f64], net: &[bool]) -> f64;   // net_pay / gross span
pub fn leverett_j(pc: f64, ift: f64, perm: f64, phi: f64) -> f64;  // (Pc/ift)·√(perm/phi), consistent units; NaN if phi≤0

// analysis::characterise — fit a Distribution + Provenance from samples (petekIO characterises, consumer samples)
pub enum DistributionShape { Normal, Triangular, LogNormal }  // Triangular = P10/P50/P90; LogNormal fitted on ln of positives
pub fn characterise(values: &[f64], shape: DistributionShape, provenance: Provenance) -> Uncertain;  // <2 defined → Deterministic

// algorithms::wells — type-light well-numeric kernels (one home per formula; the bindings call in)
pub fn dz_weights(md: &[f64]) -> Vec<f64>;                // per-sample midpoint MD-span weights (thickness weighting)

// analysis::well_tables — well-derived table/bundle crunch (pure Rust; the PyO3 bindings marshal the result)
pub enum ZoneTable { Tidy { .. }, Aggregate { .. } }      // per-(zone,bore) columns, ready for a DataFrame
pub struct NetCond<'a> { pub cut: Cutoffs, pub phi: &'a str, pub sw: &'a str, pub vsh: Option<&'a str> }  // optional net-conditioning for build_zone_table (the zone_table(cut=NetSettings) seam)
pub fn build_zone_table(bores: &[(String, &Sidetrack)], curve: &str, stats: &[&str], zones: Option<&[String]>, include_empty: bool, aggregate: bool, weighted: bool, net: Option<NetCond<'_>>) -> ZoneTable;  // net=Some → pool only net samples per cell
pub fn net_zone_samples(st: &Sidetrack, value: &str, phi: &str, sw: &str, vsh: Option<&str>, cut: &Cutoffs) -> Vec<(String, Vec<f64>)>;  // per-zone NET-conditioned kept samples
pub struct RawCurve { .. } pub struct RawZone { .. } pub struct RawWellLogs { .. }  // well.view() bundle payload
pub fn gather_raw_logs(st: &Sidetrack, kb: f64, filter: Option<&[String]>) -> RawWellLogs;  // master-MD-grid resample of a bore's logs
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

project = petekio.Project.import_data("Data", aliases={
    "por": ["PHIE", "PORO", "PorE_BC"],   # canonical -> raw names is accepted
})
project.inventory()                       # counts + loaded names + stable skipped reasons
project.surfaces["Top reservoir"]         # named access over the underlying GeoData
project.well("15/9-A1")
project.geodata                           # the underlying GeoData substrate
project.rename_surface("Top reservoir", "structure/top agat")
project.surfaces.structure.top_agat       # folder view + unique leaf lookup
project.surfaces.all_names()              # canonical names with folders
project.delete_surface("structure/top agat")
project.save("field.pproj")               # compact .pproj write
pproj_project = petekio.Project.load("field.pproj")  # compact .pproj read

geo = petekio.GeoData(unit="ft")
geo.load_surface("top", "top.irap")       # or top.CPS3grid (CPS-3 grid)
# real-format dispatch by extension: .CPS3grid→surface, .CPS3lines→polygons,
# .EarthVisionGrid/.IrapClassicPoints→points. Also as classmethods:
# Surface.load_cps3_grid, PolygonSet.load_cps3_lines, PointSet.load_earthvision_grid
top, base = geo.surface("top"), geo.surface("base")

thick = (base - top).clamp_min(0)        # operator overloads
trend = top.attr["seismic"].ln()
top.stats.p50                            # Stats fields as attributes
top.area_below(8240)
ongrid = top.resample(grid_geom)

# Geometry shells: iso-lines + value layers on ALL three surface levels
# (Surface / StructuredMeshSurface / TriSurface); NaN-aware, deterministic.
top.iso_lines(interval=25.0)             # [(level, [[(x, y), ...], ...]), ...]
top.iso_lines(levels=[1800.0, 1850.0])   # explicit levels win over interval
top.iso_lines(interval=5.0, attr="twt")  # contour an attribute lane
top.value_layer()                        # {"kind": "trimesh", "name", "nodes",
                                         #  "triangles", "values", "range"} — the
                                         #  petektools-viewer bundle (do not change)
sm = top.to_structured_mesh()            # level 1 → 2 (free; attrs carried 1:1)
tri = top.to_tri_surface()               # level 1 → 3 (quad-split; attrs carried 1:1)
sm.to_tri_surface()                      # level 2 → 3
tri.shell; sm.shell                      # the Arc-shared geometry shells (MeshShell /
                                         #  StructuredShell): nodes/triangles/labels/...
tri.infer_grid(); sm.infer_grid()        # downward fit → GridGeometry (raises if irregular)
tri.resample(grid_geom, "nearest")       # downward resample: primary + ALL attr lanes → Surface
tri.attr("amp"); tri.attr_names()        # attribute lanes on levels 2/3 mirror Surface
tri2 = tri.set_attr("amp", per_node)     # set_attr returns a NEW object (shell shared)

geo.load_well("15/9-A1", wellhead=(1200, 1500), kb=82, files="wells/A1/")
w = geo.well("15/9-A1")
w.xyz(2450)                              # interpolated position at MD

# Multi-bore wells (a Petrel export tree → one bore per .wellpath) + tops + zone stats:
geo.load_well("15/9-A1", files="wells/")  # head/kb optional — the .wellpath header fills them
geo.load_well("15/9-A1", files="wells/", ingest=petekio.IngestSpec(aliases={"PHIE_2025": "PHIE"}))  # declarative, per-call (non-sticky); then log("PHIE") resolves
geo.load_well("15/9-A1", files="wells/", aliases={"PHIE_2025": "PHIE"})  # DEPRECATED (sticky project state; DeprecationWarning) — use ingest=IngestSpec(aliases=...)
# ingest XOR aliases: passing both is a loud error.
geo.load_well_tops("WellTops.tops", ingest=petekio.IngestSpec(strat_hints=[("Base Shale", "Upper Sand")]))  # declarative order hints applied at this load
geo.strat_hint("Base Shale < Upper Sand")    # DEPRECATED (sticky) — use load_well_tops(ingest=IngestSpec(strat_hints=[...])); still: A<B = A above B, or above=/below=

# Persistence — one .pproj file (splittable/mergeable/tag-filterable):
geo.set_owner("kk"); geo.set_tags(["field-a"]); geo.set_element_tags("15/9-A1", ["field-a"])
geo.save("field.pproj")
info = petekio.GeoData.inspect("field.pproj")     # dict: owner/tags/unit/elements — no decode
geo = petekio.GeoData.open("field.pproj")
petekio.GeoData.export("field.pproj", "share.pproj", ["field-a"])   # tagged subset, one binary
petekio.GeoData.split("field.pproj", "wells.pproj", ["15/9-A1"])
petekio.GeoData.merge("a.pproj", "b.pproj", "both.pproj")
geo.put_model_section("model/seg/props", ["field-a"], 1, payload_bytes)   # petekSim's opaque sidecar
v, data = geo.model_section("model/seg/props")    # (version, bytes)
geo.load_well_tops("WellTops.tops")      # Horizon picks → well+bore; derives the strat column
geo.strat_order                          # ["Top A", "Sand A", "Top B", ...] global lithostrat column
w.crs; w.bores()                         # CRS label; e.g. ["", "A", "B", "ST2"]
w.contact("OWC"); bore.contacts()        # fluid contacts as (name, md), not formation zones
w.is_multibore                           # True → the top-level accessors below RAISE until a bore is chosen
w.xyz(2450)                              # ValueError "well '99/9-1' has 3 bores (A, B, ST2) — use .sidetrack(name) or .set_default_bore(name)"
w.set_default_bore("A"); w.default_bore  # route w.xyz/tvd/log/top through bore A ("A"); clears via load
bore = w.sidetrack("A")                  # per-bore access is first-class + complete (never needs a default):
bore.xyz(1200); bore.tvd(1200); bore.md_range()    # positioned by THIS bore's trajectory
bore.mnemonics(); bore.log_stats("PHIE").mean      # whole-bore curve + stats
bore.log("PHIE").values(); bore.log("PERM").geomean()   # per-sample view on a named bore; geometric-mean perm
lv = bore.log("PHIE"); lv.at_md_many([2400,2410,2420]); lv.values_md()  # batched: one chain-resolution for a slice / (values, md) together
bore.net_zone_stats("PHIE")                        # [(zone, Stats)] over NET samples (φ/Sw cutoff-conditioned)
bore.net_zone_stats("PHIE", cut=petekio.NetSettings(phi_min=0.10))  # a NetSettings supplies the cutoffs
bore.net_zone_stats("PHIE", cut=cut, phi_min=0.12) # scalar kwargs stay as per-call overrides ON TOP of cut
bore.net_zone_stats("PERM", geomean=True)          # [(zone, float)] net geometric-mean permeability
petekio.Stats.geomean([1.0, 4.0])                  # 2.0
ps = petekio.PointSet.from_xyz(xs, ys, zs); ps.z_stats().mean   # in-memory points + z-range
poly = petekio.PolygonSet.from_rings([[[0,0],[10,0],[10,10],[0,10]]])  # in-memory polygons
bore.zones()                             # [(name, top_md, base_md), ...] in lithostrat order
bore.zone_stats("PHIE")                  # [(name, Stats), ...] in lithostrat order
bore.zone_stats("PHIE", "Top A").mean    # one zone's Stats directly (None if absent)

# Tidy per-zone×bore table (needs pandas: pip install petekio[pandas]):
w.zone_table("PHIE", stats=("mean","p50"))            # tidy [zone, bore, mean, p50]; zone = ordered Categorical
w.zone_table("PHIE", pivot=True, decimals=3)          # wide: zone index × bore cols, rounded
w.zone_table("PHIE", aggregate=True)                  # grouped (zone,bore); pooled "all" row first per zone
w.zone_table("PHIE", stats=("mean","gross","samples")) # also: gross (zone MD thickness), samples (count)
w.zone_table("PHIE", zones=("Top A","Top B"))         # keep only these zones (case-insensitive)
# averages are thickness-weighted by default (weighted=False for plain sample mean)
w.zone_table("PHIE", cut=petekio.NetSettings(phi_min=0.10))  # net-condition each cell (phi/sw/vsh name the curves; default PHIE/SW/none — inert without cut)
geo.wells.zone_table("PHIE")                          # multi-well; bore = "<well> <sidetrack>"

# Standalone trajectory from a directional survey (no project needed):
traj = petekio.Trajectory.from_stations(      # [(md, inc_deg, azi_deg), ...]
    [(0, 0, 145), (1200, 0, 145), (1900, 57, 145)], head=(1000, 2000), kb=27.3)
traj.tvd(1655.81)                        # subsea TVD (RKB = + kb); xyz / md_at_tvd / md_range too
w.brent.ntg                             # -> Stats  (dynamic: __getattr__ top → log)
w.brent.phie.mean
geo.wells.filter(field="Gullfaks").tops("Brent")   # broadcast view

# Curve-name authority (canonical LAS mnemonic; petekio is the family namer):
petekio.canonical_mnemonic("suwi")       # -> "SW"   (case-insensitive, vintage-stripped)
petekio.detect("surface_without_extension") == petekio.FormatKind.IrapClassicGrid

# Standalone log correlation viewer — the WellLogBundle producer + a logs-only
# viewer session (kind "wells_logs", schema_version 4). Builds the bundle from a
# well's own logs + trajectory and hands it to the viewer unit (petektools.viewer,
# an OPTIONAL runtime dependency, imported lazily). Seam:
# petekSuite/dev-docs/designs/well-log-bundle-seam.md.
w.view()                                 # serve the well's logs (non-blocking); -> LogSession
w.view(tops=True)                        # include the well's tops/zones + a flatten pick
w.view(curves=("PHIE","SW"), tops=["Upper Sand"])   # select curves; a subset of tops (legacy per-call kwargs)
w.view(spec=petekio.ViewSpec(curves=("PHIE","SW"), tops=["Upper Sand"], cutoff=petekio.NetSettings(phi_min=0.10)))  # declarative WHAT
w.view(spec=petekio.ViewSpec(...), settings=petekio.ViewSettings(save="well.html"))  # WHAT + HOW; spec XOR legacy WHAT kwargs (loud on both)
w.view(save="well.html")                 # export one self-contained HTML file instead (legacy)
sess = w.view(settings=petekio.ViewSettings(serve=False)); sess.bundle()   # build only; inspect the payload dict
geo.wells.view(settings=petekio.ViewSettings(serve=False))   # multi-well logs-only session (same surface)
# legacy kwargs (spec/settings absent): curves=None, tops=None|True|[names], flatten_default=None,
#   phie_cutoff=0.08, flags=None, serve=True, save=None.  Standalone bundles carry NO ties.
petekio.build_well_log_bundle(raws, spec=petekio.ViewSpec(tops=True))   # pure-Python producer (spec= or legacy kwargs; testable)
petekio.encode_lane([0.2, None, 0.4])    # one f32 base64 lane block {dtype, shape, data}

# Spec value-objects — declarative, frozen; each: to_dict/from_dict (a "spec"-tagged
# JSON-durable dict), value equality, .replace(**overrides) derivation, domain-table repr.
petekio.NetSettings(phi_min=0.08, sw_max=0.5, vsh_max=0.5)   # φ/Sw/Vsh reservoir cutoffs (wraps core Cutoffs)
high = base.replace(phi_min=0.10)                            # a derived scenario spec
petekio.IngestSpec(aliases={"PHIE_2025":"PHIE"}, strat_hints=[("A","B"), "C < D"], unit="m")  # load-time canonicalization
petekio.ViewSpec(curves=("PHIE",), tops=True, flatten_default=None, flags=None, cutoff=0.08|NetSettings)  # WHAT view() shows
petekio.ViewSettings(serve=True, save=None)                  # HOW view() delivers
```

Python rules: `Stats` fields exposed as read-only attributes; operators (`+ - * /`)
on `Surface`; `surface.attr["name"]` indexed access; `surface.edge` and
`surface.geometry.edge` expose matching `PolygonSet` outlines; `PointSet`
exposes `infer_geometry(tolerance=1e-3, edge="full_rect", max_bridge=None, fallback="tri") -> GridGeometry | TriSurface`
and `to_structured_surface(tolerance=1e-3, edge="occupied")`, both taking
`edge="occupied"|"convex_hull"|"full_rect"`; `detect_topology(nominal_cell=None)`
returns `(points | None, TopologyReport)` whose `.verified` gates the labels;
`infer_geometry` preserves strict regular inference; when no regular lattice
describes the points it delegates to `to_tri_surface(max_link=None,
max_bridge=...)` **with a `UserWarning`** (`fallback="tri"`, the default) or
raises a `ValueError` (`fallback="error"`). `max_bridge` (in cells) applies
**only to the TriSurface fallback** — it closes boundary-fringe/fault-seam/
data-gap edges up to that length (`None` = strictly lattice-closed) and has no
effect on an inferred `GridGeometry`. Both possible results carry a
discoverable `.kind` for import-free dispatch — every geometry/surface/point
object exposes it: `"grid_geometry"` | `"surface"` | `"structured_mesh"` |
`"tri_surface"` | `"point_set"` | `"polygon_set"`.
`PointSet.to_surface(geom=None, method="idw", tolerance=1e-3) -> Surface`
grids z onto `geom`; `geom=None` (default) infers the lattice internally
(`tolerance` as in `infer_geometry`) and **raises a `ValueError`** when the
points are not lattice-regular — it never grids onto an arbitrary bounding
lattice (pass an explicit `GridGeometry` or use `to_tri_surface()`); passing
the `infer_geometry` TriSurface fallback as `geom` is a `TypeError` pointing
at `tri_surface.resample(geom, method)`;
`well.<top>.<log>` resolves via `__getattr__` (top interval → log → `Stats`).
**Dataset names (duck-typed viewer seam):** every project-accessor hand-back
(`project.points[...]`, `project.surfaces[...]`, `project.polygons[...]`,
`geo.points(name)`, `geo.surface(name)`, `geo.polygons(name)`, and the
`load_*` project loaders) carries a read-only `.name` property — the lookup
key's leaf (`"Surfaces/IrapClassic_points/Top Agat"` → `"Top Agat"`).
Derived objects propagate it: `infer_geometry`/`surface.geometry`/`infer_grid`
→ `"<name> geometry"`; `to_surface`/`to_tri_surface`/`to_structured_surface`/
`to_structured_mesh`/`to_points`/`resample`/`detect_topology`'s labelled
points/attr promotion keep `"<name>"`. Anonymous/in-memory objects
(`from_xyz`, `Surface.load_*`, arithmetic results, …) return `None`.
`TriSurface.points()`/`.xyz()` keep returning `(x, y, z)` tuples (shell XY
zipped with z); `StructuredMeshSurface`/`TriSurface` attribute access is
method-style (`attr(name)` promotes the lane on the same shared shell;
`set_attr` returns a **new** object — these types are immutable wrappers,
unlike `Surface`'s copy-on-write `set_attr`).
Bindings are thin: every method delegates to the Rust API above. The spec
value-objects (`NetSettings`, `IngestSpec`, `ViewSpec`, `ViewSettings`) follow
the family house spec pattern (declarative WHAT / HOW, applied at explicit
moments; spec XOR legacy kwargs).
