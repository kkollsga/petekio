//! Golden: a whole `GeoData` project round-trips through a single `.pproj` file
//! — manifest (owner/tags/unit/strat_order) + elements — and `inspect` lists the
//! project without decoding any element.

use petekio::{GeoData, Unit};

mod common;

#[test]
fn project_save_open_inspect_round_trip() {
    let well_dir = common::synth_well();
    let tops = common::synth_tops();
    let mut geo = GeoData::new(Unit::Metres);
    geo.load_well("99/9-X", (0.0, 0.0), 0.0, &well_dir).unwrap();
    geo.load_well_tops(&tops).unwrap();
    geo.set_owner("kkollsga");
    geo.set_tags(vec!["demo".into(), "gate-0".into()]);
    let strat = geo.strat_order().to_vec();

    let path = common::tmpdir("proj").join("field.pproj");
    geo.save(&path).unwrap();

    // inspect() reads the manifest only — no element decode.
    let info = GeoData::inspect(&path).unwrap();
    assert_eq!(info.owner.as_deref(), Some("kkollsga"));
    assert!(info.tags.contains(&"demo".to_string()));
    assert_eq!(info.unit.as_deref(), Some("Metres"));
    assert!(info.created.is_some() && info.modified.is_some());
    assert!(info
        .elements
        .iter()
        .any(|(k, n)| k == "well" && n == "99/9-X"));

    // open() materializes the project.
    let re = GeoData::open(&path).unwrap();
    assert_eq!(re.unit, Unit::Metres);
    assert_eq!(re.owner(), Some("kkollsga"));
    assert_eq!(re.tags(), ["demo", "gate-0"]);
    assert_eq!(re.strat_order(), strat.as_slice());

    let w = re.well("99/9-X").expect("well round-tripped");
    let a = w.sidetrack("A").expect("bore A round-tripped");
    assert!(!a.trajectories().is_empty()); // positioned trajectory preserved
    assert!(!a.zones().is_empty()); // tops → zones preserved
}

#[test]
fn model_sections_export_split_merge_byte_lossless() {
    let well_dir = common::synth_well();
    let mut geo = GeoData::new(Unit::Metres);
    geo.load_well("99/9-X", (0.0, 0.0), 0.0, &well_dir).unwrap();
    geo.set_element_tags("99/9-X", vec!["cerisa".into()]);
    // Two opaque model sidecars, differently tagged + versioned.
    let model = vec![0u8, 255, 7, 42, 255];
    geo.put_model_section(
        "model/cerisa/props",
        vec!["cerisa".into()],
        3,
        model.clone(),
    );
    geo.put_model_section("model/other/props", vec!["other".into()], 1, vec![9, 9, 9]);

    let dir = common::tmpdir("proj2");
    let src = dir.join("full.pproj");
    geo.save(&src).unwrap();

    // Opaque model bytes + per-section version round-trip exactly.
    let re = GeoData::open(&src).unwrap();
    assert_eq!(
        re.model_section("model/cerisa/props"),
        Some((3, model.clone()))
    );
    assert_eq!(re.model_section_names().len(), 2);

    // Export by tag → a shareable subset with only 'cerisa' sections.
    let sub = dir.join("cerisa.pproj");
    GeoData::export(&src, &sub, &["cerisa"]).unwrap();
    let s = GeoData::open(&sub).unwrap();
    assert!(s.well("99/9-X").is_some());
    assert_eq!(s.model_section_names(), vec!["model/cerisa/props"]); // 'other' dropped
    assert_eq!(
        s.model_section("model/cerisa/props"),
        Some((3, model.clone()))
    ); // byte-for-byte

    // Split by name, then merge the pieces back.
    let well_only = dir.join("well.pproj");
    GeoData::split(&src, &well_only, &["99/9-X"]).unwrap();
    assert!(GeoData::open(&well_only)
        .unwrap()
        .model_section_names()
        .is_empty());
    let merged = dir.join("merged.pproj");
    GeoData::merge(&well_only, &sub, &merged).unwrap();
    let m = GeoData::open(&merged).unwrap();
    assert!(m.well("99/9-X").is_some());
    assert_eq!(m.model_section("model/cerisa/props"), Some((3, model)));
}
