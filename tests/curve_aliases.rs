//! Opt-in curve-mnemonic canonicalization at load (Task 1d / weakness W9):
//! `set_curve_aliases` renames vintage/vendor mnemonics to canonical so
//! `log("PHIE")` resolves a `PHIE_2025` curve. Default (no aliases) preserves
//! raw mnemonics (covered by multibore_well.rs).

use petekio::{GeoData, NameMap, Unit};

mod common;
use common::synth_well;

#[test]
fn empty_alias_map_auto_canonicalizes_vintage() {
    let d = synth_well();
    let mut geo = GeoData::new(Unit::Metres);
    geo.set_curve_aliases(NameMap::new()); // pure auto-canonicalization
    geo.load_well("99/9-X", (0.0, 0.0), 0.0, &d).unwrap();
    let w = geo.well("99/9-X").unwrap();

    let a = w.sidetrack("A").unwrap();
    assert!(a.log("PHIE").is_some(), "PHIE_2025 → PHIE should resolve");
    assert!(a.log("PHIE_2025").is_none(), "raw name renamed away");
    assert!(a.log("CPOR").is_some(), "unmapped core curve preserved");
    assert!(
        w.sidetrack("ST2").unwrap().log("SW").is_some(),
        "SW_2025 → SW should resolve"
    );
}

#[test]
fn explicit_alias_map_wins() {
    let d = synth_well();
    let mut geo = GeoData::new(Unit::Metres);
    // Map the unguessable: force CPOR → PHIE (as if core porosity is the phi).
    geo.set_curve_aliases(NameMap::from_pairs([(
        "CPOR".to_string(),
        "PHIE".to_string(),
    )]));
    geo.load_well("99/9-X", (0.0, 0.0), 0.0, &d).unwrap();
    let w = geo.well("99/9-X").unwrap();
    let a = w.sidetrack("A").unwrap();
    // Both PHIE_2025 (via table) and CPOR (via explicit map) land on PHIE.
    let phies = a.logs().filter(|l| l.mnemonic == "PHIE").count();
    assert_eq!(phies, 2);
}
