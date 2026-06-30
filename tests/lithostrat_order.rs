//! Golden: the global lithostratigraphic column derived at `load_well_tops` time
//! across *every* well in the file, and pushed down so a loaded well's `zones()`
//! follow it. The field resolves an order a single borehole cannot — a marker
//! coincident (zero thickness) in one well is ordered by a well that develops it.

use petekio::{GeoData, Unit};

mod common;

#[test]
fn global_column_merges_across_all_wells_in_the_file() {
    let (_well, tops) = common::synth_field();
    let mut geo = GeoData::new(Unit::Metres);
    // No wells loaded → no picks attached, but the column still spans the file
    // (FIELD-2 develops Lower<Mid, FIELD-3 develops Sand<Mid). Sand is listed
    // last in the file yet sorts to its true depth, above Mid.
    let attached = geo.load_well_tops(&tops).unwrap();
    assert_eq!(attached, 0);
    assert_eq!(geo.strat_order(), &["Top", "Sand", "Mid", "Lower"]);
}

#[test]
fn loaded_well_zones_follow_the_global_column() {
    let (well, tops) = common::synth_field();
    let mut geo = GeoData::new(Unit::Metres);
    geo.load_well("FIELD-1", (0.0, 0.0), 0.0, &well).unwrap();
    geo.load_well_tops(&tops).unwrap();

    let w = geo.well("FIELD-1").unwrap();
    let names: Vec<_> = w.zones().iter().map(|z| z.name.clone()).collect();
    // FIELD-1 has Top, Mid, Sand with Mid/Sand coincident and Sand added last,
    // so by MD/insertion alone this would be [Top, Mid, Sand]. The cross-well
    // column lifts Sand above Mid (FIELD-3 develops Sand ≺ Mid).
    assert_eq!(names, ["Top", "Sand", "Mid"]);
}
