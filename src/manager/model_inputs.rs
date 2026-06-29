//! `GeoData::model_inputs` — assemble the model-ready inputs contract from a
//! loaded project. The single entry point consumers call; everything upstream
//! (ingest → normalize → validate → interpret → characterise) is internal.

use super::GeoData;
use crate::analysis::characterise::{characterise, DistributionShape};
use crate::analysis::interpret::{net_flags, net_pay, net_to_gross, Cutoffs};
use crate::analysis::normalize::{canonical_mnemonic, harmonise_fraction};
use crate::analysis::validate::mask_out_of_range;
use crate::analysis::{HorizonInput, ModelInputs, SpatialInputs, SummaryInputs, WellCurveInput};
use crate::core::Well;
use crate::foundation::{Provenance, Result, Uncertain, Unit};

/// Per-well petrophysical aggregates accumulated during assembly.
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

        for well in self.wells().iter() {
            self.accumulate_well(well, &cutoffs, &mut acc, &mut well_curves);
        }

        let summary = self.summary(&acc);
        let spatial = SpatialInputs {
            boundary: self.boundary_polygon(),
            horizons: self.horizons(),
            well_curves,
        };
        Ok(ModelInputs { summary, spatial })
    }

    /// Emit a [`WellCurveInput`] per curve and fold this well's net-pay
    /// petrophysics into `acc`.
    fn accumulate_well(
        &self,
        well: &Well,
        cutoffs: &Cutoffs,
        acc: &mut PetroAcc,
        well_curves: &mut Vec<WellCurveInput>,
    ) {
        for log in well.logs() {
            let mnemonic = canonical_mnemonic(&log.mnemonic);
            let md = log.md_slice().to_vec();
            let mut values: Vec<f64> = log
                .values_slice()
                .iter()
                .map(|v| harmonise_fraction(*v, &log.unit))
                .collect();
            mask_out_of_range(&mnemonic, &mut values);
            well_curves.push(WellCurveInput {
                well_id: well.id.clone(),
                mnemonic,
                md,
                values,
                provenance: Provenance::HardData,
            });
        }

        let (Some((md, phi)), Some((_, sw))) = (
            self.canonical_curve(well, "PHIE"),
            self.canonical_curve(well, "SW"),
        ) else {
            return; // no porosity/saturation pair → no net-pay contribution
        };
        let net = net_flags(&phi, &sw, None, cutoffs);
        let depth: Vec<f64> = md.iter().map(|&m| well.tvd(m).unwrap_or(m)).collect();
        acc.net_pay.push(net_pay(&depth, &net));
        acc.ntg.push(net_to_gross(&depth, &net));
        for (i, &is_net) in net.iter().enumerate() {
            if is_net {
                acc.phi_net.push(phi[i]);
                acc.sw_net.push(sw[i]);
            }
        }
    }

    /// A canonical curve `(md, harmonised+validated values)` on the well's main
    /// bore whose normalized mnemonic equals `target`, or `None`.
    fn canonical_curve(&self, well: &Well, target: &str) -> Option<(Vec<f64>, Vec<f64>)> {
        let log = well
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

    /// Characterise the summary scalars from the accumulated petrophysics.
    fn summary(&self, acc: &PetroAcc) -> SummaryInputs {
        let net_pay_ft: Vec<f64> = acc
            .net_pay
            .iter()
            .map(|&v| self.unit.convert(v, Unit::Feet))
            .collect();
        SummaryInputs {
            reservoir_area_acres: self.reservoir_area_acres(),
            net_pay_ft: characterise(
                &net_pay_ft,
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
            owc_ft: None,
            goc_ft: None,
        }
    }

    /// Reservoir area as an [`Uncertain`] in acres, from the boundary polygon's
    /// area (project unit² → acres). `NaN` deterministic when no boundary exists.
    fn reservoir_area_acres(&self) -> Uncertain {
        match self.boundary_polygon() {
            Some(poly) => Uncertain {
                value: self.unit.area_to_acres(poly.area()),
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
    fn boundary_polygon(&self) -> Option<crate::core::PolygonSet> {
        if let Some((_, poly)) = self.polygons_named().next() {
            return Some(poly.clone());
        }
        self.surfaces().next().and_then(|s| s.boundary_polygon())
    }
}
