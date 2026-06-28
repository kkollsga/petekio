//! Golden tests: load the committed LAS + tops CSV fixtures, assert mnemonics /
//! units / NULL→NaN mapping / the index curve, and the tops→interval→stats
//! chain end to end on a constructed well.

use petekio::{Log, Top, TrajectoryInput, Well};

const LAS: &str = "tests/fixtures/sample.las";
const TOPS: &str = "tests/fixtures/tops.csv";

#[test]
fn las_curves_units_and_null_mapping() {
    let logs = Log::load_las_all(LAS).unwrap();
    // Two non-index curves: GR, NTG.
    let names: Vec<&str> = logs.iter().map(|l| l.mnemonic.as_str()).collect();
    assert_eq!(names, ["GR", "NTG"]);

    let gr = logs.iter().find(|l| l.mnemonic == "GR").unwrap();
    assert_eq!(gr.unit, "GAPI");
    // Index/MD curve carried onto every log.
    assert_eq!(gr.view().md(), &[2400.0, 2410.0, 2420.0, 2430.0, 2440.0]);
    // The -999.25 NULL at 2410 maps to NaN.
    let v = gr.view();
    assert_eq!(v.values()[0], 45.0);
    assert!(v.values()[1].is_nan());
    // Stats skip the NULL → 4 defined samples.
    assert_eq!(v.stats().count, 4);

    let ntg = logs.iter().find(|l| l.mnemonic == "NTG").unwrap();
    assert_eq!(ntg.unit, "v/v");
}

#[test]
fn load_single_curve_by_mnemonic() {
    let ntg = Log::load_las(LAS, "ntg").unwrap(); // case-insensitive
    assert_eq!(ntg.mnemonic, "NTG");
    assert_eq!(ntg.len(), 5);
    assert!(Log::load_las(LAS, "RHOB").is_err()); // absent
}

#[test]
fn tops_csv_loads_by_column() {
    let tops = Top::load_csv(TOPS, "name", "md").unwrap();
    assert_eq!(tops.len(), 3);
    assert_eq!(tops[0], Top::new("Brent", 2400.0));
    assert_eq!(tops[2].md, 2500.0);
    assert!(Top::load_csv(TOPS, "name", "nope").is_err()); // missing column
}

#[test]
fn end_to_end_well_top_log_stats() {
    let mut w = Well::new("15/9-A1", (0.0, 0.0), 82.0);
    let st = w.sidetrack_mut("");
    st.add_trajectory(TrajectoryInput::Stations(vec![
        petekio::Station::new(2400.0, 0.0, 0.0),
        petekio::Station::new(2460.0, 0.0, 0.0),
    ]))
    .unwrap();
    for log in Log::load_las_all(LAS).unwrap() {
        st.add_log(log);
    }
    st.add_tops(Top::load_csv(TOPS, "name", "md").unwrap());

    // Brent = [2400, 2450): NTG samples 2400..2440 → 0.1..0.5, mean 0.3.
    let stats = w.top("Brent").unwrap().log("NTG").unwrap().stats();
    assert_eq!(stats.count, 5);
    assert!((stats.mean - 0.3).abs() < 1e-12);
}
