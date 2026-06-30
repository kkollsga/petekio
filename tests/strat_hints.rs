//! Soft lithostratigraphic hints: break stalemates the data can't resolve, while
//! real MD positions always win. Shorthand (`"A < B"`) + partial-name resolution
//! + explicit `add_strat_hint(above, below)`.

use petekio::{GeoData, Unit};
use std::fs;

mod common;

const HEADER: &str = "# Petrel well tops\nVERSION 2\nBEGIN HEADER\nX\nY\nZ\nTWT\nTWT2\nage\nMD\nPVD\nType\nSurface\nWell\nEND HEADER\n";

fn tops(rows: &str) -> std::path::PathBuf {
    let d = common::tmpdir("hints");
    let p = d.join("w.tops");
    fs::write(&p, format!("{HEADER}{rows}")).unwrap();
    p
}

/// One well: `Alpha top`/`Beta top` coincident at 120 (a stalemate the data
/// can't order); `Top` strictly above all, `Deep` strictly below all.
fn body() -> &'static str {
    "1 2 -1 -999 -999 -999 100.0 -1 Horizon \"Top\" \"W1\"\n\
     1 2 -1 -999 -999 -999 120.0 -1 Horizon \"Alpha top\" \"W1\"\n\
     1 2 -1 -999 -999 -999 120.0 -1 Horizon \"Beta top\" \"W1\"\n\
     1 2 -1 -999 -999 -999 200.0 -1 Horizon \"Deep\" \"W1\"\n"
}

#[test]
fn hint_breaks_stalemate_with_partial_names() {
    let p = tops(body());
    // Default: the coincident pair falls to first-appearance (Alpha then Beta).
    let mut geo = GeoData::new(Unit::Metres);
    geo.load_well_tops(&p).unwrap();
    assert_eq!(geo.strat_order(), &["Top", "Alpha top", "Beta top", "Deep"]);

    // Shorthand + partial names: "Beta above Alpha" → reorders only the tie.
    let mut geo = GeoData::new(Unit::Metres);
    geo.strat_hint("Beta < Alpha").unwrap();
    geo.load_well_tops(&p).unwrap();
    assert_eq!(geo.strat_order(), &["Top", "Beta top", "Alpha top", "Deep"]);
}

#[test]
fn explicit_kwargs_form_is_equivalent() {
    let p = tops(body());
    let mut geo = GeoData::new(Unit::Metres);
    geo.add_strat_hint("Beta top", "Alpha top"); // exact names, above/below
    geo.load_well_tops(&p).unwrap();
    assert_eq!(geo.strat_order(), &["Top", "Beta top", "Alpha top", "Deep"]);
}

#[test]
fn hint_cannot_override_data() {
    let p = tops(body());
    let mut geo = GeoData::new(Unit::Metres);
    geo.strat_hint("Deep < Top").unwrap(); // "Deep above Top" — but data says Top ≺ Deep
    geo.load_well_tops(&p).unwrap();
    let o = geo.strat_order();
    let pos = |n: &str| o.iter().position(|s| s == n).unwrap();
    assert!(pos("Top") < pos("Deep")); // data wins; the hint is dropped
}

#[test]
fn ambiguous_token_errors_at_load() {
    let p = tops(
        "1 2 -1 -999 -999 -999 100.0 -1 Horizon \"Top\" \"W1\"\n\
                  1 2 -1 -999 -999 -999 120.0 -1 Horizon \"Upper Sand top\" \"W1\"\n\
                  1 2 -1 -999 -999 -999 130.0 -1 Horizon \"Lower Sand top\" \"W1\"\n",
    );
    let mut geo = GeoData::new(Unit::Metres);
    geo.strat_hint("Sand < Top").unwrap(); // parses; "Sand" matches two tops
    assert!(geo.load_well_tops(&p).is_err());
}

#[test]
fn bad_spec_errors() {
    let mut geo = GeoData::new(Unit::Metres);
    assert!(geo.strat_hint("no operator").is_err()); // neither < nor >
    assert!(geo.strat_hint("< Top").is_err()); // empty side
}
