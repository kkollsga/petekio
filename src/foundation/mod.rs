//! `foundation` — the bottom layer: errors, units, geometry primitives, and the
//! universal `Stats` aggregation result. Imports from nothing above it.

pub mod error;
pub mod geometry;
pub mod history;
pub mod stats;
pub mod units;
pub mod uncertain;

pub use error::{GeoError, Result};
pub use geometry::{BBox, GridGeometry, Point3};
pub use history::{HasHistory, OperationHistory};
pub use stats::Stats;
pub use uncertain::{Distribution, Provenance, Uncertain};
pub use units::Unit;
