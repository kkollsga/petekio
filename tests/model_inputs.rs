//! Golden integration test for `GeoData::model_inputs` — the model-ready-inputs
//! contract. Loads a small project (one depth surface + one petrophysics well
//! whose curves use vendor aliases `PHI`/`SUWI` in percent) and asserts the
//! assembled `ModelInputs` against hand calculations: net pay, average φ/Sw over
//! pay, net-to-gross, canonical well curves, horizons, and the boundary.

use petekio::{GeoData, Provenance, Unit};

const SURFACE: &str = "tests/fixtures/simple.irap";
const WELL_DIR: &str = "tests/fixtures/wells_petro/15_9-A1";

fn project() -> GeoData {
    let mut geo = GeoData::new(Unit::Feet);
    geo.load_surface("top", SURFACE).unwrap();
    geo.load_well("15/9-A1", (1200.0, 1500.0), 0.0, WELL_DIR)
        .unwrap();
    geo
}

#[test]
fn summary_net_pay_and_averages_hand_calc() {
    let mi = project().model_inputs().unwrap();
    let s = &mi.summary;

    // Vertical well, kb 0, samples every 10 ft over [2400, 2440].
    // PHI = [.20,.05,.20,.20,.20]; SUWI% = [30,30,80,30,30] → SW = /100.
    // Cutoffs φ≥0.08, Sw≤0.5 → net = [T,F,F,T,T].
    // Voronoi thickness [5,10,10,10,5]; net pay = 5+10+5 = 20 ft.
    assert!(
        (s.net_pay_ft.value - 20.0).abs() < 1e-9,
        "net_pay={}",
        s.net_pay_ft.value
    );
    assert_eq!(s.net_pay_ft.provenance, Provenance::Interpolated);

    // gross span 40 → NTG = 0.5.
    assert!((s.net_to_gross_frac.value - 0.5).abs() < 1e-9);

    // Average φ/Sw over net samples (idx 0,3,4): φ all 0.20, Sw all 0.30.
    assert!((s.porosity_frac.value - 0.20).abs() < 1e-9);
    assert!((s.water_saturation_frac.value - 0.30).abs() < 1e-9);
    assert_eq!(s.porosity_frac.provenance, Provenance::HardData);

    // No fluid-contact data in the fixture.
    assert!(s.owc_ft.is_none() && s.goc_ft.is_none());
}

#[test]
fn spatial_horizons_and_boundary() {
    let mi = project().model_inputs().unwrap();
    let sp = &mi.spatial;

    // One surface → one horizon, named, flagged Interpolated.
    assert_eq!(sp.horizons.len(), 1);
    assert_eq!(sp.horizons[0].name, "top");
    assert_eq!(sp.horizons[0].provenance, Provenance::Interpolated);

    // Boundary derived from the surface's defined region; area > 0.
    assert!(sp.boundary.is_some());
    assert!(mi.summary.reservoir_area_acres.value > 0.0);
    assert_eq!(
        mi.summary.reservoir_area_acres.provenance,
        Provenance::Interpolated
    );
}

#[test]
fn well_curves_are_canonical_and_harmonised() {
    let mi = project().model_inputs().unwrap();
    let curves = &mi.spatial.well_curves;

    // PHI/SUWI/NTG → canonical PHIE/SW/NTG, all on well 15/9-A1.
    assert!(curves.iter().all(|c| c.well_id == "15/9-A1"));
    let mnemonics: Vec<&str> = curves.iter().map(|c| c.mnemonic.as_str()).collect();
    assert!(mnemonics.contains(&"PHIE"));
    assert!(mnemonics.contains(&"SW"));
    assert!(mnemonics.contains(&"NTG"));

    // SW came in as percent → harmonised to fraction (and 0.80 is in range).
    let sw = curves.iter().find(|c| c.mnemonic == "SW").unwrap();
    assert!((sw.values[0] - 0.30).abs() < 1e-9);
    assert!((sw.values[2] - 0.80).abs() < 1e-9);
    assert_eq!(sw.provenance, Provenance::HardData);
    assert_eq!(sw.md.len(), 5);

    // Positioned to world (x,y,z=TVD): well head (1200,1500), kb 0, vertical →
    // first sample at md 2400 → (1200, 1500, 2400).
    assert_eq!(sw.xyz.len(), sw.md.len());
    assert!((sw.xyz[0][0] - 1200.0).abs() < 1e-9);
    assert!((sw.xyz[0][1] - 1500.0).abs() < 1e-9);
    assert!((sw.xyz[0][2] - 2400.0).abs() < 1e-9);
}
