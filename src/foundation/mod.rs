//! `foundation` — the bottom layer: errors, units, geometry primitives, and the
//! universal `Stats` aggregation result. Imports from nothing above it.

pub mod error;
pub mod units;
pub mod geometry;
pub mod stats;

pub use error::{GeoError, Result};
pub use geometry::{BBox, GridGeometry, Point3};
pub use stats::Stats;
pub use units::Unit;
