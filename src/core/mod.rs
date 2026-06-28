//! `core` — the domain objects that carry their own operations: `Surface`,
//! `Well`/`Sidetrack`/`Trajectory`, `Log`, `Top`, `PointSet`, `PolygonSet`.
//! Imports from `foundation` and `io`.

mod gridding; // scattered-point → Surface interpolation (Nearest/IDW/min-curvature)
pub mod log; // Log + LogView — MD-indexed well curves and views
pub mod points; // PointSet — scattered points + attributes + gridding
pub mod polygons; // PolygonSet — rings + contains/area/bbox/clip
pub mod surface;
mod surface_ops; // arithmetic + operator overloads on Surface
mod surface_stats; // statistics + volumetrics on Surface
pub mod tops; // Top → Interval — formation tops and the depth interval each names
pub mod trajectory; // well path: minimum-curvature normalization + interpolation
pub mod well; // Well → Sidetrack → Trajectory hierarchy

pub use log::{Log, LogView};
pub use points::{GridMethod, PointSet};
pub use polygons::PolygonSet;
pub use surface::Surface;
pub use tops::{Interval, Top};
pub use trajectory::{Station, Trajectory, TrajectoryInput};
pub use well::{Sidetrack, Well};
