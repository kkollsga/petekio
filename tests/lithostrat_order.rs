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
    let bore = w
        .sidetrack("A")
        .expect("bore A carries the trajectory + picks");
    let zones: Vec<_> = bore
        .zones()
        .iter()
        .map(|z| (z.name.clone(), z.top_md, z.base_md))
        .collect();
    let names: Vec<_> = zones.iter().map(|(n, _, _)| n.clone()).collect();
    // FIELD-1 has Top, Mid, Sand with Mid/Sand coincident and Sand added last,
    // so by MD/insertion alone this would be [Top, Mid, Sand]. The cross-well
    // column lifts Sand above Mid (FIELD-3 develops Sand ≺ Mid).
    assert_eq!(names, ["Top", "Sand", "Mid"]);
    // Tie-break base assignment: in the {Mid, Sand} coincident cluster, Mid is
    // the deeper (Sand ≺ Mid), so Mid owns the interval down to TD and Sand
    // pinches to zero — not the arbitrary insertion-order pick.
    let geom = |name: &str| {
        zones
            .iter()
            .find(|(n, _, _)| n == name)
            .map(|(_, t, b)| (*t, *b))
    };
    assert_eq!(geom("Sand"), Some((120.0, 120.0))); // shallower coincident → zero
    assert!(geom("Mid").unwrap().1 > 120.0); // deeper owns the interval to TD
}

#[test]
fn petrel_other_picks_surface_as_contacts_not_zones() {
    let (well, _tops) = common::synth_field();
    let d = common::tmpdir("fluid_contacts");
    let tops = d.join("contacts.tops");
    std::fs::write(
        &tops,
        "# Petrel well tops\nVERSION 2\nBEGIN HEADER\nX\nY\nZ\nTWT\nTWT2\nage\nMD\nPVD\nType\nSurface\nWell\nEND HEADER\n\
         1 2 -1 -999 -999 -999 100.0 -1 Horizon \"Top\" \"FIELD-1 A\"\n\
         1 2 -1 -999 -999 -999 130.0 -1 Other \"OWC\" \"FIELD-1 A\"\n",
    )
    .unwrap();

    let mut geo = GeoData::new(Unit::Metres);
    geo.load_well("FIELD-1", (0.0, 0.0), 0.0, &well).unwrap();
    let attached_tops = geo.load_well_tops(&tops).unwrap();
    assert_eq!(attached_tops, 1);

    let bore = geo.well("FIELD-1").unwrap().sidetrack("A").unwrap();
    assert_eq!(bore.zones().len(), 1);
    let contact = bore.contact("owc").unwrap();
    assert_eq!(contact.name, "OWC");
    assert_eq!(contact.md, 130.0);
}
