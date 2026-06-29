//! Golden: a multi-sidetrack well organizes from a folder of `.wellpath` +
//! per-bore `.las`. Asserts bores, positioned trajectories (MD preserved,
//! z = TVD − kb), per-bore log routing, and recorded header (KB/XY/CRS).

use petekio::{GeoData, Unit};

const WELL_DIR: &str = "tests/fixtures/wells_multibore/36_7-X";

#[test]
fn multibore_well_organizes_from_wellpaths() {
    let mut geo = GeoData::new(Unit::Metres);
    // Pass placeholder head/kb — the .wellpath header is authoritative.
    geo.load_well("36/7-X", (0.0, 0.0), 0.0, WELL_DIR).unwrap();
    let w = geo.well("36/7-X").unwrap();

    // Header taken from the wellpath, not the placeholder call args.
    assert_eq!(w.head, (558650.0, 6812460.0));
    assert!((w.kb - 27.3).abs() < 1e-9);
    assert!(w.crs().unwrap().contains("UTM"));

    // Two bores: A and ST2 (labels = stem minus shared "36_7-X_" prefix).
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
}
