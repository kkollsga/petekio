//! Golden: a single **deviated** sidetrack (one `.wellpath` + one comp-log +
//! tops) positions its logs/tops through its one trajectory regardless of bore
//! naming — the single-trajectory routing rule. Asserts *actual* xyz along the
//! deviation (not just non-empty), correct datum/tvd/elevation, positioned
//! model-ready curves (`WellCurveInput.xyz`), and variant-tolerant well-name
//! matching for Petrel tops.

use petekio::{GeoData, Unit};

mod common;

#[test]
fn single_deviated_well_positions_curves_through_one_trajectory() {
    let dir = common::synth_deviated_well();
    let mut geo = GeoData::new(Unit::Metres);
    // Placeholder head/kb — the .wellpath header is authoritative.
    geo.load_well("99/9-1 A", (0.0, 0.0), 0.0, &dir).unwrap();
    let w = geo.well("99/9-1 A").unwrap();

    // One wellpath → the single (main) bore; the header datum is authoritative.
    let labels: Vec<&str> = w.sidetracks().map(|s| s.label.as_str()).collect();
    assert_eq!(labels, vec![""], "single wellpath → main bore only");
    assert_eq!(w.head, (1000.0, 2000.0));
    assert!((w.kb - 27.3).abs() < 1e-9);

    // The comp-log co-located onto the same bore as the one trajectory.
    assert!(w.log("GR").is_some());

    // Positioned along the deviation: by MD 1000 the path is already 600 m east
    // of the wellhead (x ≈ 1600, NOT the head's 1000) — a real deviation, not a
    // vertical projection. z = kb - TVD (negative-down elevation); tvd = TVD - kb.
    // Tolerance 0.05 absorbs the fixture's rounded inclination (asin(0.6) to 4 dp).
    let p0 = w.xyz(1000.0).unwrap();
    assert!((p0.x - 1600.0).abs() < 0.05, "x={}", p0.x);
    assert!((p0.y - 2000.0).abs() < 1e-9);
    assert!((p0.z - (27.3 - 800.0)).abs() < 0.05, "z={}", p0.z);
    assert!((w.tvd(1000.0).unwrap() - (800.0 - 27.3)).abs() < 0.05);

    let p2 = w.xyz(1020.0).unwrap();
    assert!((p2.x - 1612.0).abs() < 0.05, "x={}", p2.x);
    assert!((p2.z - (27.3 - 816.0)).abs() < 0.05);

    // Tops (CSV) co-located on the same bore; the deepest runs to the
    // trajectory's TD (1020) — proving `top` resolves through the one trajectory.
    assert_eq!(w.top("Brent").unwrap().base_md, 1010.0); // next top
    assert_eq!(w.top("Dunlin").unwrap().base_md, 1020.0); // → TD

    // The model-ready curve is positioned via WellCurveInput.xyz — the acceptance
    // path. Every sample has a finite world position along the deviation (the
    // defect left these NaN: the log sat on the main bore, the trajectory on a
    // differently-named bore, so `well.xyz` could not position it).
    let mi = geo.model_inputs().unwrap();
    let gr = mi
        .spatial
        .well_curves
        .iter()
        .find(|c| c.well_id == "99/9-1 A" && c.mnemonic == "GR")
        .expect("GR curve present");
    assert_eq!(gr.xyz.len(), 3);
    assert!(
        gr.xyz.iter().all(|p| p.iter().all(|v| v.is_finite())),
        "every sample positioned: {:?}",
        gr.xyz
    );
    assert!((gr.xyz[0][0] - 1600.0).abs() < 0.05); // md 1000 → x ≈ 1600
    assert!((gr.xyz[0][2] - (27.3 - 800.0)).abs() < 0.05);
    assert!((gr.xyz[2][0] - 1612.0).abs() < 0.05); // md 1020 → x ≈ 1612
                                                   // Interior sample (md 1010) lies along the deviated arc — x strictly between.
    assert!(
        gr.xyz[1][0] > 1600.0 && gr.xyz[1][0] < 1612.0,
        "interior x={}",
        gr.xyz[1][0]
    );
}

#[test]
fn petrel_tops_match_well_name_variant() {
    // The well id uses `/`, `-`, and a space; the tops `Well` field uses `_`
    // throughout. Variant-tolerant matching still routes the pick to the well
    // (and its single main bore), where it resolves through the one trajectory.
    let dir = common::synth_deviated_well();
    let tops = common::synth_variant_tops();
    let mut geo = GeoData::new(Unit::Metres);
    geo.load_well("99/9-1 A", (0.0, 0.0), 0.0, &dir).unwrap();
    let added = geo.load_well_tops(&tops).unwrap();
    assert_eq!(added, 1, "variant-named pick matched and routed");
    let w = geo.well("99/9-1 A").unwrap();
    assert_eq!(w.top("Base").unwrap().top_md, 1015.0);
}
