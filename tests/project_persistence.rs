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
fn structured_surface_round_trips_in_whole_project() {
    let dir = common::tmpdir("structured_project");
    let source = dir.join("top.EarthVisionGrid");
    std::fs::write(
        &source,
        "# Type: scattered data\n# Grid_size: 2 x 2\n# Null_value: 1.0e30\n# End:\n\
         0 0 10 1 1\n10 0 11 2 1\n0 10 1.0e30 1 2\n10 10 13 2 2\n",
    )
    .unwrap();

    let mut geo = GeoData::new(Unit::Metres);
    geo.load_structured_surface("top", &source).unwrap();
    assert!(geo
        .model_inputs()
        .err()
        .expect("structured horizons must not be silently omitted")
        .to_string()
        .contains("top"));
    let project = dir.join("structured.pproj");
    geo.save(&project).unwrap();

    let info = GeoData::inspect(&project).unwrap();
    assert!(info
        .elements
        .iter()
        .any(|(kind, name)| kind == "structured_mesh" && name == "top"));
    let reopened = GeoData::open(&project).unwrap();
    let surface = reopened.structured_surface("top").unwrap();
    assert_eq!((surface.ncol(), surface.nrow()), (2, 2));
    assert_eq!(surface.node_xy(0, 1).unwrap(), (0.0, 10.0));
    assert!(surface.z(0, 1).unwrap().is_nan());
}

#[test]
fn model_sections_export_split_merge_byte_lossless() {
    let well_dir = common::synth_well();
    let mut geo = GeoData::new(Unit::Metres);
    geo.load_well("99/9-X", (0.0, 0.0), 0.0, &well_dir).unwrap();
    geo.set_element_tags("99/9-X", vec!["field-a".into()]);
    // Two opaque model sidecars, differently tagged + versioned.
    let model = vec![0u8, 255, 7, 42, 255];
    geo.put_model_section(
        "model/field-a/props",
        vec!["field-a".into()],
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
        re.model_section("model/field-a/props"),
        Some((3, model.clone()))
    );
    assert_eq!(re.model_section_names().len(), 2);

    // Export by tag → a shareable subset with only 'field-a' sections.
    let sub = dir.join("field-a.pproj");
    GeoData::export(&src, &sub, &["field-a"]).unwrap();
    let s = GeoData::open(&sub).unwrap();
    assert!(s.well("99/9-X").is_some());
    assert_eq!(s.model_section_names(), vec!["model/field-a/props"]); // 'other' dropped
    assert_eq!(
        s.model_section("model/field-a/props"),
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
    assert_eq!(m.model_section("model/field-a/props"), Some((3, model)));
}

#[test]
fn generic_assets_are_separate_versioned_and_byte_lossless() {
    let mut geo = GeoData::new(Unit::Metres);
    let envelope = br#"{"asset_type":"mystery","codec":"application/octet-stream","future":{"x":1},"provider":"example.Future","schema_version":7}"#.to_vec();
    let bytes = vec![0, 255, 9, 0, 42];
    geo.add_asset(
        "@asset/future/nested/value",
        "mystery",
        envelope.clone(),
        vec!["share".into()],
        1,
        bytes.clone(),
    )
    .unwrap();
    assert!(geo
        .add_asset(
            "@asset/future/nested/value",
            "mystery",
            envelope.clone(),
            vec![],
            1,
            vec![],
        )
        .unwrap_err()
        .to_string()
        .contains("already exists"));

    let dir = common::tmpdir("assets");
    let first = dir.join("first.pproj");
    geo.save(&first).unwrap();
    let info = GeoData::inspect(&first).unwrap();
    assert!(info.elements.contains(&(
        "asset".to_string(),
        "@asset/future/nested/value".to_string()
    )));

    let mut reopened = GeoData::open(&first).unwrap();
    let asset = reopened.asset("@asset/future/nested/value").unwrap();
    assert_eq!(asset.kind, "mystery");
    assert_eq!(asset.version, 1);
    assert_eq!(asset.tags, ["share"]);
    assert_eq!(asset.envelope, envelope);
    assert_eq!(asset.bytes, bytes);

    reopened
        .rename_asset("@asset/future/nested/value", "@asset/future/nested/renamed")
        .unwrap();
    let second = dir.join("second.pproj");
    reopened.save(&second).unwrap();
    let twice = GeoData::open(&second).unwrap();
    let asset = twice.asset("@asset/future/nested/renamed").unwrap();
    assert_eq!(asset.envelope, envelope);
    assert_eq!(asset.bytes, bytes);

    let split = dir.join("split.pproj");
    GeoData::split(&second, &split, &["future/nested/renamed"]).unwrap();
    assert!(GeoData::open(&split)
        .unwrap()
        .asset("@asset/future/nested/renamed")
        .is_some());
}

#[test]
fn generic_asset_names_and_envelopes_are_strict() {
    let mut geo = GeoData::new(Unit::Metres);
    let envelope = br#"{"asset_type":"template","codec":"application/json","provider":"petektools.viewer.CorrelationTemplate","schema_version":1}"#.to_vec();
    for bad in ["templates/x", "@asset/templates/../x", "@asset//x"] {
        assert!(geo
            .add_asset(bad, "template", envelope.clone(), vec![], 1, vec![])
            .is_err());
    }
    let noncanonical = br#"{"provider": "x", "asset_type":"template","codec":"application/json","schema_version":1}"#.to_vec();
    assert!(geo
        .add_asset(
            "@asset/templates/x",
            "template",
            noncanonical,
            vec![],
            1,
            vec![],
        )
        .unwrap_err()
        .to_string()
        .contains("canonical"));

    geo.put_model_section("@asset/templates/collision", vec![], 1, vec![]);
    let path = common::tmpdir("asset_collision").join("bad.pproj");
    assert!(geo
        .save(path)
        .unwrap_err()
        .to_string()
        .contains("reserved prefix"));
}
