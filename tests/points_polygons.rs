//! Golden tests for Phase 5 IO: GeoJSON/IRAP/CSV round-trips for `PointSet`
//! (incl. the property-carrying `FeatureProcessor`) and `PolygonSet`.

use petekio::{GridGeometry, GridMethod, PointSet, PolygonSet};

/// A 1×1 grid whose only node sits exactly on a sample, so a Nearest grid
/// returns that sample's Z — used to confirm Z survived the loader.
fn node_geom(x: f64, y: f64) -> GridGeometry {
    GridGeometry {
        xori: x,
        yori: y,
        xinc: 1.0,
        yinc: 1.0,
        ncol: 1,
        nrow: 1,
        rotation_deg: 0.0,
        yflip: false,
    }
}

#[test]
fn geojson_points_carry_numeric_properties() {
    let p = PointSet::load_geojson("tests/fixtures/points.geojson").unwrap();
    assert_eq!(p.len(), 3);
    // numeric property carried into a column; string "name" dropped.
    assert_eq!(p.attr("poro").unwrap(), &[0.10, 0.20, 0.30]);
    assert!(p.attr("name").is_none());
    let s = p.stats("poro").unwrap();
    approx::assert_relative_eq!(s.mean, 0.20);
    // Z made it through: Nearest grid at the (10,0) node → 20.
    let surf = p
        .to_surface(node_geom(10.0, 0.0), GridMethod::Nearest)
        .unwrap();
    approx::assert_relative_eq!(surf.values()[[0, 0]], 20.0);
    // areal nearest to (9,1) is the (10,0) sample, index 1.
    assert_eq!(p.nearest(9.0, 1.0), Some(1));
}

#[test]
fn csv_points_named_columns_and_attrs() {
    let p = PointSet::load_csv("tests/fixtures/points.csv", "x", "y", "depth").unwrap();
    assert_eq!(p.len(), 3);
    assert_eq!(p.attr("poro").unwrap(), &[0.10, 0.20, 0.30]);
    // "well" is non-numeric → dropped; x/y/depth are not attributes.
    assert!(p.attr("well").is_none());
    assert!(p.attr("x").is_none());
    let surf = p
        .to_surface(node_geom(0.0, 10.0), GridMethod::Nearest)
        .unwrap();
    approx::assert_relative_eq!(surf.values()[[0, 0]], 30.0); // Z from "depth"
}

#[test]
fn irap_points_load_xyz() {
    let p = PointSet::load_irap_points("tests/fixtures/points.xyz").unwrap();
    assert_eq!(p.len(), 3);
    let surf = p
        .to_surface(node_geom(0.0, 0.0), GridMethod::Nearest)
        .unwrap();
    approx::assert_relative_eq!(surf.values()[[0, 0]], 10.0);
}

#[test]
fn geojson_polygon_area_and_contains() {
    let poly = PolygonSet::load_geojson("tests/fixtures/square.geojson").unwrap();
    approx::assert_relative_eq!(poly.area(), 1.0);
    assert!(poly.contains(0.5, 0.5));
    assert!(!poly.contains(0.0, 0.0)); // boundary excluded
    let b = poly.bbox();
    approx::assert_relative_eq!(b.xmax, 1.0);
}

#[test]
fn irap_polygon_area_matches_unit_square() {
    let poly = PolygonSet::load_irap_polygons("tests/fixtures/square.pol").unwrap();
    approx::assert_relative_eq!(poly.area(), 1.0);
    assert!(poly.contains(0.5, 0.5));
}
