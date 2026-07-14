//! Extension-dispatch of the real-format readers (task_petekio_real_readers)
//! through `GeoData`: CPS-3 grid → surface, CPS-3 lines → polygons, EarthVision
//! grid + `.IrapClassicPoints` → points. All fixtures hand-authored to spec.

use petekio::{GeoData, Unit};

mod common;
use common::tmpdir;

fn write(path: &std::path::Path, body: &str) {
    std::fs::write(path, body).unwrap();
}

#[test]
fn cps3_grid_surface_dispatch() {
    let d = tmpdir("cps3grid");
    let p = d.join("trend.CPS3grid");
    write(
        &p,
        "FSASCI 0 1 0 5 1.0E+30\n\
         FSLIMI 100 110 200 220 0 6\n\
         FSNROW 3 2\n\
         FSXINC 10 10\n\
         ->\n1 2\n3 1.0E+30\n5 6\n",
    );
    let mut geo = GeoData::new(Unit::Metres);
    geo.load_surface("trend", &p).unwrap();
    let s = geo.surface("trend").unwrap();
    assert_eq!((s.geom.ncol, s.geom.nrow), (2, 3));
    // South-west node = (xmin, ymin); value grid is row-major, first row = south.
    assert!(!s.geom.yflip);
    assert_eq!(s.geom.node_xy(0, 0), (100.0, 200.0)); // (xmin, ymin)
    assert_eq!(s.values()[[0, 0]], 1.0);
    assert_eq!(s.values()[[1, 2]], 6.0);
    assert!(s.values()[[1, 1]].is_nan());
}

#[test]
fn cps3_lines_polygons_dispatch() {
    let d = tmpdir("cps3lines");
    let p = d.join("outline.CPS3lines");
    write(
        &p,
        "FFASCI 0 1 2 3\n\
         -> 1\n0.0 0.0 -10.0\n10.0 0.0 -10.0\n10.0 10.0 -10.0\n0.0 10.0 -10.0\n",
    );
    let mut geo = GeoData::new(Unit::Metres);
    geo.load_polygons("outline", &p).unwrap();
    let poly = geo.polygons("outline").unwrap();
    assert_eq!(poly.rings().len(), 1);
    approx::assert_relative_eq!(poly.area(), 100.0);
    assert!(poly.contains(5.0, 5.0));
}

#[test]
fn earthvision_grid_points_dispatch() {
    let d = tmpdir("evgrid");
    let p = d.join("netsand.EarthVisionGrid");
    write(
        &p,
        "# Type: scattered data\n# Grid_size: 2 x 2\n# Null_value: 1.0e30\n# End:\n\
         100.0 200.0 0.5\n110.0 200.0 0.6\n100.0 210.0 1.0e30\n110.0 210.0 0.7\n",
    );
    let mut geo = GeoData::new(Unit::Metres);
    geo.load_structured_surface("surface", &p).unwrap();
    let surface = geo.structured_surface("surface").unwrap();
    assert_eq!((surface.ncol(), surface.nrow()), (2, 2));
    assert!(surface.z(0, 1).unwrap().is_nan());
    assert!(geo
        .load_surface("surface", "tests/fixtures/simple.irap")
        .err()
        .expect("cross-kind duplicate surface name must fail")
        .to_string()
        .contains("already belongs"));

    // Deprecated compatibility view remains finite scattered nodes only.
    geo.load_points("netsand", &p).unwrap();
    assert_eq!(geo.points("netsand").unwrap().len(), 3); // null node dropped
}

#[test]
fn irapclassicpoints_extension_dispatch() {
    let d = tmpdir("irappts");
    let p = d.join("horizon.IrapClassicPoints");
    write(&p, "1.0 2.0 -100.0\n3.0 4.0 -110.0\n5.0 6.0 -120.0\n");
    let mut geo = GeoData::new(Unit::Metres);
    geo.load_points("horizon", &p).unwrap();
    assert_eq!(geo.points("horizon").unwrap().len(), 3);
}
