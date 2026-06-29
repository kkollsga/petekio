//! Golden: a multi-sidetrack well organizes from a folder of `.wellpath` +
//! per-bore `.las`. Asserts bores, positioned trajectories (MD preserved,
//! z = TVD − kb), per-bore log routing, and recorded header (KB/XY/CRS).

use petekio::{GeoData, LogKind, Unit};

mod common;

#[test]
fn multibore_well_organizes_from_wellpaths() {
    let Some(well_dir) = common::require("wells_multibore/99_9-X") else {
        return;
    };
    let mut geo = GeoData::new(Unit::Metres);
    // Pass placeholder head/kb — the .wellpath header is authoritative.
    geo.load_well("99/9-X", (0.0, 0.0), 0.0, &well_dir).unwrap();
    let w = geo.well("99/9-X").unwrap();

    // Header taken from the wellpath, not the placeholder call args.
    assert_eq!(w.head, (1000.0, 2000.0));
    assert!((w.kb - 27.3).abs() < 1e-9);
    assert!(w.crs().unwrap().contains("UTM"));

    // Two bores: A and ST2 (labels = stem minus shared "99_9-X_" prefix).
    let labels: Vec<&str> = w.sidetracks().map(|s| s.label.as_str()).collect();
    assert!(labels.contains(&"A"), "labels={labels:?}");
    assert!(labels.contains(&"ST2"), "labels={labels:?}");

    // Positioned trajectory: MD preserved, z = TVD − kb (subsea).
    let a = w.sidetrack("A").unwrap();
    assert!((a.tvd(1200.0).unwrap() - (1200.0 - 27.3)).abs() < 1e-6);
    assert_eq!(a.active().md_range(), (0.0, 2000.0));

    // Logs routed by filename token: PHIE on A, SW on ST2.
    assert!(w.sidetrack("A").unwrap().log("PHIE_2025").is_some());
    assert!(w.sidetrack("ST2").unwrap().log("SW_2025").is_some());
    assert!(w.sidetrack("A").unwrap().log("SW_2025").is_none()); // not cross-routed

    // Core LAS (`..._full_core.las`) ingested onto bore A and tagged Core;
    // comp-logs stay Log. Lets a consumer include/exclude core per zone.
    let a_bore = w.sidetrack("A").unwrap();
    let cpor = a_bore.logs().find(|l| l.mnemonic == "CPOR").unwrap();
    assert_eq!(cpor.kind(), LogKind::Core);
    let phie = a_bore.logs().find(|l| l.mnemonic == "PHIE_2025").unwrap();
    assert_eq!(phie.kind(), LogKind::Log);
}

#[test]
fn petrel_tops_route_to_well_and_bore() {
    let Some(well_dir) = common::require("wells_multibore/99_9-X") else {
        return;
    };
    let tops = common::require("wells_multibore/wells_tops.tops").unwrap();
    let mut geo = GeoData::new(Unit::Metres);
    geo.load_well("99/9-X", (0.0, 0.0), 0.0, &well_dir).unwrap();
    let added = geo.load_well_tops(&tops).unwrap();
    // 2 valid picks land (the -999-MD row + the unknown well "99/9-Z" are skipped).
    assert_eq!(added, 2);
    let w = geo.well("99/9-X").unwrap();
    // "Top A" picked on bore A at MD 1210, and on ST2 at MD 1510.
    assert_eq!(
        w.sidetrack("A").unwrap().top("Top A").unwrap().top_md,
        1210.0
    );
    assert_eq!(
        w.sidetrack("ST2").unwrap().top("Top A").unwrap().top_md,
        1510.0
    );
}

#[test]
fn split_layout_recursion_and_id_filter() {
    // Real Petrel layout: Paths/ + Logs/ in separate subdirs, and a foreign
    // well's log that must NOT be ingested into 99/9-Y.
    let Some(root) = common::require("wells_split") else {
        return;
    };
    let mut geo = GeoData::new(Unit::Metres);
    geo.load_well("99/9-Y", (0.0, 0.0), 0.0, &root).unwrap();
    let w = geo.well("99/9-Y").unwrap();
    // Two bores found across the split dirs; the foreign 99_9-OTHER log excluded.
    let labels: Vec<&str> = w.sidetracks().map(|s| s.label.as_str()).collect();
    assert!(
        labels.contains(&"A") && labels.contains(&"B"),
        "labels={labels:?}"
    );
    assert!(w.sidetrack("A").unwrap().log("GR").is_some());
    // The foreign 99_9-OTHER log was filtered out, so it did NOT fall back onto
    // the main bore (which has no wellpath/logs of its own here).
    assert_eq!(w.sidetrack("").unwrap().logs().count(), 0);
}
