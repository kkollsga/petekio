//! `core` — the domain objects that carry their own operations: `Surface`,
//! `Well`/`Sidetrack`/`Trajectory`, `Log`, `Top`, `PointSet`, `PolygonSet`.
//! Imports from `foundation` and `io`.

pub mod log; // Log + LogView — MD-indexed well curves and views
pub mod intersection; // typed trajectory/surface crossings
pub(crate) mod persist; // per-element save/load to a standalone .pproj
pub mod points; // PointSet — scattered points + attributes + gridding
pub mod polygons; // PolygonSet — rings + contains/area/bbox/clip
pub mod shell; // the three-level geometry-shell system (flat empty shells)
pub mod surface;
pub mod structured_surface;
mod surface_convert; // cross-level surface conversions (up free, down via gridding)
mod surface_filter; // NaN-aware smoothing + boundary polygon on Surface
mod surface_ops; // arithmetic + operator overloads on Surface
mod surface_stats; // statistics + volumetrics on Surface
pub mod topology; // (column,row) recovery from unlabelled surface points
pub mod tri_surface; // TriSurface — the triangulated fallback for fault-cut surfaces
pub mod tops; // Top → Interval — formation tops and the depth interval each names
pub mod trajectory; // well path: minimum-curvature normalization + interpolation
pub mod value_layer; // iso-lines + value layers (trimesh views) on all surface levels
pub mod well; // Well → Sidetrack → Trajectory hierarchy

pub use intersection::{IntersectableSurface, SurfaceIntersection};
pub use log::{Log, LogKind, LogView};
pub use points::{GeometryEdge, GridMethod, PointSet};
pub use polygons::PolygonSet;
pub use shell::{CornerTable, MeshShell, StructuredShell, WalkLabel, NO_CORNER};
pub use structured_surface::StructuredMeshSurface;
pub use surface::Surface;
pub use topology::TopologyReport;
pub use tops::{FluidContact, Interval, Top};
pub use trajectory::{Station, Trajectory, TrajectoryInput};
pub use tri_surface::{TriSurface, DEFAULT_MAX_LINK};
pub use value_layer::ValueLayer;
pub use well::{Sidetrack, Well};
pub mod attribute;
pub use attribute::{AttributeKind, AttributeMetadata, CodeRecord};
