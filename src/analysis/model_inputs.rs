//! `ModelInputs` — the model-ready-inputs contract petekio delivers to
//! consumers (petekSim and friends).
//!
//! Everything here has already been ingested, **normalized** (aliases,
//! name-maps, units), **validated** (bounds / validity-range), **interpreted**
//! (petrophysics → net_pay/φ/Sw), and **characterised** for uncertainty. The
//! consumer maps these straight into its domain and derives nothing.
//!
//! Two granularities: [`SummaryInputs`] (scalars, as [`Uncertain`]) for
//! box-model & Monte-Carlo volumetrics, and [`SpatialInputs`] (surfaces +
//! curves) for the 3D grid build and upscaling. The consumer hands a target
//! lattice to [`Surface::resample`](crate::Surface::resample) to put horizons on
//! its grid.

use crate::core::{PolygonSet, Surface};
use crate::foundation::{Provenance, Uncertain};

/// The model-ready inputs assembled from a project.
pub struct ModelInputs {
    pub summary: SummaryInputs,
    pub spatial: SpatialInputs,
}

/// Scalar inputs, each an [`Uncertain`] in **canonical units** (documented per
/// field). These feed the box-model and the Monte-Carlo volumetrics.
pub struct SummaryInputs {
    /// Reservoir / drainage area, acres.
    pub reservoir_area_acres: Uncertain,
    /// Net pay thickness, ft (cut-off-derived from logs).
    pub net_pay_ft: Uncertain,
    /// Effective porosity, fraction.
    pub porosity_frac: Uncertain,
    /// Water saturation, fraction.
    pub water_saturation_frac: Uncertain,
    /// Net-to-gross, fraction.
    pub net_to_gross_frac: Uncertain,
    /// Oil-water contact depth, ft (if defined).
    pub owc_ft: Option<Uncertain>,
    /// Gas-oil / gas-water contact depth, ft (if defined).
    pub goc_ft: Option<Uncertain>,
}

/// Spatial inputs for the 3D grid build + upscaling.
pub struct SpatialInputs {
    /// Areal boundary (drainage outline).
    pub boundary: Option<PolygonSet>,
    /// Depth-structure horizons, gridded and resampleable to the consumer's
    /// lattice (`top`/`base` first; intermediates follow).
    pub horizons: Vec<HorizonInput>,
    /// Interpreted log curves along each well's trajectory (φ/Sw/facies), for
    /// upscaling onto grid cells.
    pub well_curves: Vec<WellCurveInput>,
}

/// A named depth-structure horizon surface.
pub struct HorizonInput {
    pub name: String,
    pub surface: Surface,
    pub provenance: Provenance,
}

/// An interpreted, MD-indexed log curve (canonical mnemonic) along a well.
pub struct WellCurveInput {
    pub well_id: String,
    /// Canonical (post-normalize) mnemonic, e.g. `"PHIE"`, `"SW"`.
    pub mnemonic: String,
    pub md: Vec<f64>,
    pub values: Vec<f64>,
    pub provenance: Provenance,
}
