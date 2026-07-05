//! Golden tests for the `GeoData` manager substrate + `WellsView`: load a small
//! project (surfaces + wells) from the committed fixtures, then exercise named
//! lookup (hit + miss), collection iteration, and the broadcast view —
//! `filter` / `tops` narrowing and a per-well `Stats` reduction over the result.

use petekio::{GeoData, Unit};

const IRAP: &str = "tests/fixtures/simple.irap";
const WELL_DIR: &str = "tests/fixtures/wells/15_9-A1";
const LAS: &str = "tests/fixtures/sample.las";

/// A loaded two-surface, two-well project. The second well (`15/9-B2`) is loaded
/// from a bare LAS (logs only, no tops file) so it lacks the `Brent` marker.
fn project() -> GeoData {
    let mut geo = GeoData::new(Unit::Metres);
    geo.load_surface("top", IRAP).unwrap();
    geo.load_surface("base", IRAP).unwrap();
    geo.load_well("15/9-A1", (1200.0, 1500.0), 82.0, WELL_DIR)
        .unwrap();
    geo.load_well("15/9-B2", (3000.0, 4000.0), 50.0, LAS)
        .unwrap();
    geo
}

#[test]
fn named_lookup_hits_and_misses() {
    let geo = project();
    assert!(geo.surface("top").is_some());
    assert!(geo.surface("base").is_some());
    assert!(geo.surface("missing").is_none()); // miss → None
    assert_eq!(geo.well("15/9-A1").unwrap().head, (1200.0, 1500.0));
    assert!(geo.well("nope").is_none());
}

#[test]
fn collections_iterate_all_in_order() {
    let geo = project();
    assert_eq!(geo.surfaces().count(), 2);
    let ids: Vec<&str> = geo.wells().iter().map(|w| w.id.as_str()).collect();
    assert_eq!(ids, ["15/9-A1", "15/9-B2"]); // insertion order
}

#[test]
fn wells_view_filter_narrows() {
    let geo = project();
    // Filter on a well predicate (eastern wellhead).
    let east = geo.wells().filter(|w| w.head.0 > 2000.0);
    assert_eq!(east.len(), 1);
    assert_eq!(east.iter().next().unwrap().id, "15/9-B2");
}

#[test]
fn wells_view_tops_keeps_only_wells_with_that_top() {
    let geo = project();
    let brent = geo.wells().tops("Brent");
    assert_eq!(brent.len(), 1); // only A1 has tops
    assert_eq!(brent.iter().next().unwrap().id, "15/9-A1");
    // A marker no well carries → empty view.
    assert!(geo.wells().tops("Nonsuch").is_empty());
}

#[test]
fn broadcast_per_well_stats_over_filtered_view() {
    let geo = project();
    // The headline broadcast: each well's top("Brent").log("NTG").stats(),
    // collected over only the wells that have the marker — no per-item loop in
    // caller code beyond the view's own iter.
    let means: Vec<(String, f64)> = geo
        .wells()
        .tops("Brent")
        .iter()
        .filter_map(|w| {
            let stats = w.top("Brent")?.log("NTG")?.stats();
            Some((w.id.clone(), stats.mean))
        })
        .collect();

    assert_eq!(means.len(), 1);
    assert_eq!(means[0].0, "15/9-A1");
    // Brent = [2400, 2450): NTG 0.1..0.5 → mean 0.3.
    assert!((means[0].1 - 0.3).abs() < 1e-12);
}
