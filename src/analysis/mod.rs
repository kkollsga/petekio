//! `analysis` — operations across the core objects (resample, minimum-curvature,
//! statistics, filters, arithmetic; much lives as methods on the core types per
//! the design constitution) **and** the model-ready-inputs assembly: the
//! normalize → validate → interpret → characterise pipeline that turns loaded
//! data into the [`ModelInputs`] contract consumers receive.
//!
//! Imports from `foundation`, `io`, `core`.

pub mod characterise;
pub mod interpret;
pub mod model_inputs;
pub mod normalize;
pub mod validate;
pub mod well_tables; // zone_table / net_zone_stats / well.view() crunch kernels

pub use characterise::DistributionShape;
pub use interpret::Cutoffs;
pub use model_inputs::{HorizonInput, ModelInputs, SpatialInputs, SummaryInputs, WellCurveInput};
pub use normalize::{NameMap, StratHints};
