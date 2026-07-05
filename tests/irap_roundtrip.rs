//! Golden test: load the committed IRAP-classic fixture, assert geometry +
//! values (including the undefined node), then save → reload → numeric-stable.

use petekio::Surface;

const FIXTURE: &str = "tests/fixtures/simple.irap";

#[test]
fn load_fixture_geometry_and_values() {
    let s = Surface::load_irap_classic(FIXTURE).unwrap();
    let g = &s.geom;
    assert_eq!(g.ncol, 3);
    assert_eq!(g.nrow, 4);
    assert_eq!(g.xinc, 50.0);
    assert_eq!(g.yinc, 25.0);
    assert_eq!(g.rotation_deg, 30.0);
    assert_eq!(g.xori, 1000.0);
    assert_eq!(g.yori, 2000.0);
    assert!(!g.yflip);

    let v = s.values();
    assert_eq!(v.dim(), (3, 4));
    // column-major, x-fastest: first value is the origin node (i=0, j=0)
    assert_eq!(v[[0, 0]], 1000.0);
    assert_eq!(v[[1, 0]], 1010.0);
    assert_eq!(v[[2, 0]], 1020.0);
    assert_eq!(v[[0, 1]], 1005.0);
    assert!(v[[2, 1]].is_nan()); // the 9999900.0 undefined node
    assert_eq!(v[[0, 2]], 1008.0);
    assert_eq!(v[[2, 3]], 1032.0);
}

#[test]
fn save_reload_is_stable() {
    let s = Surface::load_irap_classic(FIXTURE).unwrap();
    let tmp = std::env::temp_dir().join("petekio_irap_roundtrip.irap");
    s.save_irap_classic(&tmp).unwrap();
    let s2 = Surface::load_irap_classic(&tmp).unwrap();

    assert_eq!(s.geom, s2.geom);
    let (v, v2) = (s.values(), s2.values());
    assert_eq!(v.dim(), v2.dim());
    for (a, b) in v.iter().zip(v2.iter()) {
        if a.is_nan() {
            assert!(b.is_nan());
        } else {
            assert_eq!(a, b);
        }
    }
    let _ = std::fs::remove_file(&tmp);
}
