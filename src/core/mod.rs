//! `core` — the domain objects that carry their own operations: `Surface`,
//! `Well`/`Sidetrack`/`Trajectory`, `Log`, `Top`, `PointSet`, `PolygonSet`.
//! Imports from `foundation` and `io`.

pub mod surface;
mod surface_ops; // arithmetic + operator overloads on Surface
mod surface_stats; // statistics + volumetrics on Surface
pub mod trajectory; // well path: minimum-curvature normalization + interpolation

pub use surface::Surface;
pub use trajectory::{Station, Trajectory, TrajectoryInput};
