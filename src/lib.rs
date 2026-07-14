//! # petekIO
//!
//! The subsurface **data ingestion + structure layer**: surfaces, wells,
//! points, and polygons, with loading, interpolation, filters, and statistics
//! built in. It is the input-data model that subsurface apps consume so they do
//! zero parsing/interpolation themselves.
//!
//! See `SPEC.md` (design constitution + architecture) and `API.md` (the locked
//! public API contract) at the repo root.
//!
//! ## Layered, one-way dependencies
//!
//! ```text
//! foundation → io → core → analysis → manager → py
//! ```
//!
//! A layer imports only from below — never sideways, never up. The public types
//! are re-exported at the crate root; users reach for `petekio::Surface`, not
//! `petekio::core::Surface`.

pub mod foundation;
pub mod algorithms;
pub(crate) mod io;
pub mod core;
pub mod analysis;
pub mod manager;

// Public types are re-exported at the crate root.
pub use analysis::{
    Cutoffs, DistributionShape, HorizonInput, ModelInputs, NameMap, SpatialInputs, StratHints,
    SummaryInputs, WellCurveInput,
};
pub use core::{
    AttributeKind, AttributeMetadata, CodeRecord, CornerTable, FluidContact, GeometryEdge,
    GridMethod, IntersectableSurface, Interval, Log, LogKind, LogView, MeshShell, PointSet,
    PolygonSet, Sidetrack, Station, StructuredMeshSurface, StructuredShell, Surface,
    SurfaceIntersection, Top, TopologyReport, Trajectory, TrajectoryInput, TriSurface, ValueLayer,
    WalkLabel, Well, DEFAULT_MAX_LINK, NO_CORNER,
};
pub use foundation::{
    BBox, Distribution, GeoError, GridGeometry, HasHistory, OperationHistory, Point3, Provenance,
    Result, Stats, Uncertain, Unit,
};
pub use io::detect::{detect, FormatKind};
pub use manager::{
    GeoData, IntersectionDiagnostic, ProjectInfo, WellIntersectionSet, WellTopRow, WellsView,
};
