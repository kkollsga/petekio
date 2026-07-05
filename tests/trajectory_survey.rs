//! Golden: a worked deviation survey (build → hold → turn) reproduced via
//! minimum-curvature, checked against an independently-computed survey table.
//! TVD here is RKB depth (`tvd()` is subsea TVDSS; RKB = TVDSS + kb).

use petekio::{Station, TrajectoryInput, Well};

/// The survey: (MD, INC°, AZI°). head = (1000, 2000), kb = 27.3.
fn survey_well() -> Well {
    let stations = vec![
        Station::new(0.0, 0.0, 145.0),
        Station::new(1200.0, 0.0, 145.0),
        Station::new(1900.0, 57.0, 145.0),
        Station::new(2200.0, 57.0, 145.0),
        Station::new(2500.0, 80.0, 135.0),
        Station::new(3700.0, 80.0, 135.0),
        Station::new(3900.0, 89.0, 135.0),
        Station::new(4400.0, 89.0, 135.0),
    ];
    let mut w = Well::new("SURVEY", (1000.0, 2000.0), 27.3);
    w.sidetrack_mut("")
        .add_trajectory(TrajectoryInput::Stations(stations))
        .unwrap();
    w
}

#[test]
fn min_curvature_reproduces_survey_table() {
    let w = survey_well();
    let kb = 27.3;
    // (MD, RKB TVD, NS, EW) from the reference survey table.
    let rows = [
        (1200.0, 1200.000, 0.0, 0.0),
        (1900.0, 1790.116, -262.462, 183.778),
        (2200.0, 1953.507, -468.562, 328.090),
        (2500.0, 2062.961, -679.361, 507.505),
        (3700.0, 2271.339, -1515.001, 1343.142),
        (3900.0, 2290.489, -1655.627, 1483.768),
        (4400.0, 2299.215, -2009.128, 1837.267),
    ];
    for (md, tvd_rkb, ns, ew) in rows {
        let p = w.xyz(md).unwrap();
        // xyz.z is negative-down elevation → RKB TVD = kb - z.
        assert!(
            (kb - p.z - tvd_rkb).abs() < 0.05,
            "MD {md}: TVD {} vs {tvd_rkb}",
            kb - p.z
        );
        // NS = ΔY (northing), EW = ΔX (easting) from the wellhead.
        assert!((p.y - 2000.0 - ns).abs() < 0.5, "MD {md} NS");
        assert!((p.x - 1000.0 - ew).abs() < 0.5, "MD {md} EW");
    }
}

#[test]
fn mid_station_tvd_matches_arc_ground_truth() {
    // Ground-truth RKB TVD (densified min-curvature reference) at mid-segment MDs.
    // petekIO must follow the arc, not lerp node positions (which was ~40 m off).
    let w = survey_well();
    let kb = 27.3;
    let truth = [
        (1655.81395, 1624.59611),
        (1769.76744, 1709.51138),
        (1851.16279, 1762.11764),
        (2245.0, 1976.88204),
        (2365.0, 2027.57597),
    ];
    for (md, tvd_rkb) in truth {
        let got = w.tvd(md).unwrap() + kb;
        assert!(
            (got - tvd_rkb).abs() < 0.05,
            "MD {md}: TVD {got} vs {tvd_rkb}"
        );
    }
}

#[test]
fn vertical_section_is_exact() {
    // Above the first build, tvd = md - kb (the degenerate vertical case).
    let w = survey_well();
    assert!((w.tvd(600.0).unwrap() - (600.0 - 27.3)).abs() < 1e-9);
}
