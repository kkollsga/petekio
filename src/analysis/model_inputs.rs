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
    /// Reservoir / drainage area, **m²** (base SI). (Since 0.3.0; was
    /// `reservoir_area_acres` in acres.)
    pub area_m2: Uncertain,
    /// Net pay thickness, **m** (cut-off-derived from logs). (Since 0.3.0; was
    /// `net_pay_ft` in feet.)
    pub net_pay_m: Uncertain,
    /// Effective porosity, fraction.
    pub porosity_frac: Uncertain,
    /// Water saturation, fraction.
    pub water_saturation_frac: Uncertain,
    /// Net-to-gross, fraction.
    pub net_to_gross_frac: Uncertain,
    /// Oil-water contact, **positive-down depth in metres** (if defined). A
    /// *depth*, not an elevation: deeper = larger, matching the consumer
    /// (petekStatic) `Contact.depth_m` datum — geometry z stays negative-down
    /// elevation (see [`WellCurveInput::xyz`]), but scalar contacts are depths,
    /// named `_depth_m` to keep the sign unambiguous. (Since 0.3.0; was
    /// `owc_ft`.)
    pub owc_depth_m: Option<Uncertain>,
    /// Gas-oil / gas-water contact, **positive-down depth in metres** (if
    /// defined; shallower than [`Self::owc_depth_m`]). Same datum as
    /// `owc_depth_m`. (Since 0.3.0; was `goc_ft`.)
    pub goc_depth_m: Option<Uncertain>,
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

/// An interpreted, MD-indexed log curve (canonical mnemonic) along a well,
/// positioned to world `(x, y, z)` — `z` is **negative-down elevation** (subsea),
/// the same convention as `Surface` z (see [`WellCurveInput::xyz`]) — so a
/// consumer can upscale it onto grid cells without touching the trajectory
/// (positioning is petekio's job).
pub struct WellCurveInput {
    pub well_id: String,
    /// Canonical (post-normalize) mnemonic, e.g. `"PHIE"`, `"SW"`.
    pub mnemonic: String,
    pub md: Vec<f64>,
    pub values: Vec<f64>,
    /// World position `[x, y, z]` of each sample, aligned 1:1 with `md`/`values`.
    /// **`z` is negative-down elevation** (subsea) — the same convention as
    /// `Surface` z, so a consumer positions curves against horizons with no sign
    /// flip. (Since 0.3.0; was positive-down TVDSS.) `[NaN; 3]` where the
    /// trajectory can't position an MD.
    pub xyz: Vec<[f64; 3]>,
    pub provenance: Provenance,
}
