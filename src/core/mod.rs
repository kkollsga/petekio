//! `core` — the domain objects that carry their own operations: `Surface`,
//! `Well`/`Sidetrack`/`Trajectory`, `Log`, `Top`, `PointSet`, `PolygonSet`.
//! Imports from `foundation` and `io`.

pub mod surface;
mod surface_ops; // arithmetic + operator overloads on Surface
mod surface_stats; // statistics + volumetrics on Surface

pub use surface::Surface;
