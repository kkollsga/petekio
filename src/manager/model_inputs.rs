//! `GeoData::model_inputs` — assemble the model-ready inputs contract from a
//! loaded project. The single entry point consumers call; everything upstream
//! (ingest → normalize → validate → interpret → characterise) is internal.

use super::GeoData;
use crate::analysis::characterise::{characterise, DistributionShape};
use crate::analysis::interpret::{net_flags, net_pay, net_to_gross, Cutoffs};
use crate::analysis::normalize::{canonical_mnemonic, harmonise_fraction};
use crate::analysis::validate::mask_out_of_range;
use crate::analysis::{HorizonInput, ModelInputs, SpatialInputs, SummaryInputs, WellCurveInput};
use crate::core::{Sidetrack, Well};
use crate::foundation::{Provenance, Result, Uncertain, Unit};

/// A canonical curve `(md, harmonised+validated values)` on `bore` whose
/// normalized mnemonic equals `target`, or `None`.
fn canonical_curve(bore: &Sidetrack, target: &str) -> Option<(Vec<f64>, Vec<f64>)> {
    let log = bore
        .logs()
        .find(|l| canonical_mnemonic(&l.mnemonic) == target)?;
    let md = log.md_slice().to_vec();
    let mut values: Vec<f64> = log
        .values_slice()
        .iter()
        .map(|v| harmonise_fraction(*v, &log.unit))
        .collect();
    mask_out_of_range(target, &mut values);
    Some((md, values))
}

/// Per-bore petrophysical aggregates accumulated during assembly (one entry per
/// bore that carries a φ/Sw pair — each bore is an independent penetration).
#[derive(Default)]
struct PetroAcc {
    /// Net pay thickness per well, in the project length unit.
    net_pay: Vec<f64>,
    /// Net-to-gross per well.
    ntg: Vec<f64>,
    /// Pooled porosity samples over net-pay intervals (all wells).
    phi_net: Vec<f64>,
    /// Pooled water-saturation samples over net-pay intervals (all wells).
    sw_net: Vec<f64>,
}

impl GeoData {
    /// Assemble the [`ModelInputs`] this project can offer: normalized,
    /// validated, interpreted, unit-canonical, uncertainty-carrying, and
    /// provenance-flagged. The consumer derives nothing from raw data.
    ///
    /// Conventions: every surface becomes a [`HorizonInput`] (named, flagged
    /// [`Interpolated`](Provenance::Interpolated) — a gridded product); every
    /// well log becomes a normalized + validated [`WellCurveInput`]; summary
    /// scalars are characterised across the wells with default [`Cutoffs`]; the
    /// boundary is the first polygon, or the first surface's convex outline.
    pub fn model_inputs(&self) -> Result<ModelInputs> {
        let cutoffs = Cutoffs::default();
        let mut acc = PetroAcc::default();
        let mut well_curves = Vec::new();

        // Each **bore** is an independent positioned "well" to the geomodel: emit
        // one curve-set per bore (positioned by that bore's own trajectory) and
        // fold each bore's net-pay petrophysics separately. A multi-sidetrack well
        // (99/9-1 A/B/ST2) thus surfaces every bore instead of the historical
        // single-bore-or-empty. The empty main bore of such a well contributes
        // nothing (no logs) and is skipped naturally.
        for well in self.wells().iter() {
            for bore in well.sidetracks() {
                self.accumulate_bore(well, bore, &cutoffs, &mut acc, &mut well_curves);
            }
        }

        let summary = self.summary(&acc);
        let spatial = SpatialInputs {
            boundary: self.edge_polygon(),
            horizons: self.horizons(),
            well_curves,
        };
        Ok(ModelInputs { summary, spatial })
    }

    /// Emit a [`WellCurveInput`] per curve on this **bore** — positioned by the
    /// bore's own trajectory, under the bore-qualified id `"<well> <bore>"` (or
    /// just the well id for the main bore) — and fold the bore's net-pay
    /// petrophysics into `acc`. A bore with no logs contributes nothing.
    fn accumulate_bore(
        &self,
        well: &Well,
        bore: &Sidetrack,
        cutoffs: &Cutoffs,
        acc: &mut PetroAcc,
        well_curves: &mut Vec<WellCurveInput>,
    ) {
        let well_id = well.bore_id(&bore.label);
        for log in bore.logs() {
            let mnemonic = canonical_mnemonic(&log.mnemonic);
            let md = log.md_slice().to_vec();
            let mut values: Vec<f64> = log
                .values_slice()
                .iter()
                .map(|v| harmonise_fraction(*v, &log.unit))
                .collect();
            mask_out_of_range(&mnemonic, &mut values);
            // Position each sample via THIS bore's trajectory (positioning is
            // ours); z = negative-down elevation (matches Surface z).
            let xyz: Vec<[f64; 3]> = md
                .iter()
                .map(|&m| match bore.xyz(m) {
                    Some(p) => [p.x, p.y, p.z],
                    None => [f64::NAN; 3],
                })
                .collect();
            well_curves.push(WellCurveInput {
                well_id: well_id.clone(),
                mnemonic,
                md,
                values,
                xyz,
                provenance: Provenance::HardData,
            });
        }

        let (Some((md, phi)), Some((_, sw))) =
            (canonical_curve(bore, "PHIE"), canonical_curve(bore, "SW"))
        else {
            return; // no porosity/saturation pair on this bore → no net-pay
        };
        let net = net_flags(&phi, &sw, None, cutoffs);
        let depth: Vec<f64> = md.iter().map(|&m| bore.tvd(m).unwrap_or(m)).collect();
        acc.net_pay.push(net_pay(&depth, &net));
        acc.ntg.push(net_to_gross(&depth, &net));
        for (i, &is_net) in net.iter().enumerate() {
            if is_net {
                acc.phi_net.push(phi[i]);
                acc.sw_net.push(sw[i]);
            }
        }
    }

    /// Characterise the summary scalars from the accumulated petrophysics.
    fn summary(&self, acc: &PetroAcc) -> SummaryInputs {
        // Per-well net pay converted project-unit → metres, then characterised.
        // Converting the samples pre-characterise builds the Normal natively in
        // metres — an exact location-scale rescale by k = metres_per_unit
        // (location and scale both scale by k; the shape is preserved).
        let net_pay_m: Vec<f64> = acc
            .net_pay
            .iter()
            .map(|&v| self.unit.convert(v, Unit::Metres))
            .collect();
        SummaryInputs {
            area_m2: self.area_m2(),
            net_pay_m: characterise(
                &net_pay_m,
                DistributionShape::Normal,
                Provenance::Interpolated,
            ),
            porosity_frac: characterise(
                &acc.phi_net,
                DistributionShape::Normal,
                Provenance::HardData,
            ),
            water_saturation_frac: characterise(
                &acc.sw_net,
                DistributionShape::Normal,
                Provenance::HardData,
            ),
            net_to_gross_frac: characterise(
                &acc.ntg,
                DistributionShape::Normal,
                Provenance::Interpolated,
            ),
            owc_depth_m: None,
            goc_depth_m: None,
        }
    }

    /// Reservoir area as an [`Uncertain`] in **m²**, from the boundary polygon's
    /// area (project unit² → m²). `NaN` deterministic when no boundary exists.
    fn area_m2(&self) -> Uncertain {
        match self.edge_polygon() {
            Some(poly) => Uncertain {
                value: self.unit.area_to_m2(poly.area()),
                distribution: crate::foundation::Distribution::Deterministic,
                provenance: Provenance::Interpolated,
            },
            None => Uncertain::defaulted(f64::NAN),
        }
    }

    /// Every surface as a named [`HorizonInput`], flagged `Interpolated`.
    fn horizons(&self) -> Vec<HorizonInput> {
        self.surfaces_named()
            .map(|(name, surface)| HorizonInput {
                name: name.to_string(),
                surface: surface.clone(),
                provenance: Provenance::Interpolated,
            })
            .collect()
    }

    /// The boundary outline: the first polygon set if any, else the first
    /// surface's convex outline.
    fn edge_polygon(&self) -> Option<crate::core::PolygonSet> {
        if let Some((_, poly)) = self.polygons_named().next() {
            return Some(poly.clone());
        }
        self.surfaces().next().and_then(|s| s.edge())
    }
}
