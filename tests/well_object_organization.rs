//! Capstone golden (ingest P6): a real-shaped multi-bore well organizes into a
//! GeoData carrying *all* its data — bores with positioned trajectories, every
//! curve (raw mnemonics preserved; canonical resolvable), core tagged, header
//! (KB/XY/CRS) recorded, tops routed per bore — and per-zone aggregation computes.
//! Calculations (model_inputs/petrophysics) are deliberately NOT exercised here.

use petekio::analysis::normalize::canonical_mnemonic;
use petekio::{GeoData, LogKind, Unit};

mod common;

#[test]
fn well_object_is_fully_organized() {
    let Some(well_dir) = common::require("wells_multibore/99_9-X") else {
        return;
    };
    let tops = common::require("wells_multibore/wells_tops.tops").unwrap();
    let mut geo = GeoData::new(Unit::Metres);
    geo.load_well("99/9-X", (0.0, 0.0), 0.0, &well_dir).unwrap();
    geo.load_well_tops(&tops).unwrap();
    let w = geo.well("99/9-X").unwrap();

    // Header captured from the .wellpath (authoritative), incl. CRS.
    assert_eq!(w.head, (1000.0, 2000.0));
    assert!((w.kb - 27.3).abs() < 1e-9);
    assert!(w.crs().unwrap().contains("UTM"));

    // All bores present, each positioned (trajectory resolves z = TVD − kb).
    let bores: Vec<&str> = w.sidetracks().map(|s| s.label.as_str()).collect();
    assert!(
        bores.contains(&"A") && bores.contains(&"ST2"),
        "bores={bores:?}"
    );
    let a = w.sidetrack("A").unwrap();
    assert!((a.tvd(1200.0).unwrap() - (1200.0 - 27.3)).abs() < 1e-6);

    // All curves preserved with raw mnemonics; canonical is a non-destructive
    // lookup (completeness: nothing dropped, nothing renamed in place).
    assert!(a.log("PHIE_2025").is_some());
    assert_eq!(canonical_mnemonic("PHIE_2025"), "PHIE");

    // Core kept + tagged distinct from logs.
    let cpor = a.logs().find(|l| l.mnemonic == "CPOR").unwrap();
    assert_eq!(cpor.kind(), LogKind::Core);

    // Tops routed to the bore; per-zone aggregation computes.
    let zs = a.zone_stats("PHIE_2025");
    let cerisa = zs.iter().find(|(z, _)| z == "Top A").unwrap();
    // Zone [1210, TD): PHIE samples 1210/1220 → 0.22/0.18 → mean 0.20.
    assert!(
        (cerisa.1.mean - 0.20).abs() < 1e-9,
        "mean={}",
        cerisa.1.mean
    );
}
