//! Y-orientation golden tests for the grid-surface readers (CPS-3, IRAP,
//! EarthVision) — task_petekio_cps3_yflip.
//!
//! The same **asymmetric** synthetic surface (a feature that climbs monotonically
//! toward the NORTH edge, so a Y-flip is unmistakable — a symmetric fixture could
//! not catch one) is written in each format to its own spec, ingested through the
//! public readers, and asserted **node-for-node equal in world coordinates**.
//! IRAP is the reference-correct baseline (§`irap.rs`): first stored value = the
//! origin node at (xmin, ymin), i.e. the SOUTH row. A spec-correct CPS-3 grid
//! stores its first data row at that same SOUTH edge (Golden Software CPS-3 /
//! Petrel dialect: values run bottom-up), so the two must agree.

use petekio::{GeoData, GridGeometry, GridMethod, Unit};

mod common;
use common::tmpdir;

const NCOL: usize = 3; // X nodes
const NROW: usize = 4; // Y nodes
const XORI: f64 = 431000.0;
const YORI: f64 = 6521000.0; // south edge (ymin)
const INC: f64 = 100.0;

/// The reference lattice — origin at the SOUTH-WEST node, Y increasing north.
fn ref_geom() -> GridGeometry {
    GridGeometry {
        xori: XORI,
        yori: YORI,
        xinc: INC,
        yinc: INC,
        ncol: NCOL,
        nrow: NROW,
        rotation_deg: 0.0,
        yflip: false,
    }
}

/// Reference z at node (i=col, j=row-from-south): climbs +100 per row north,
/// +1 per column east. Node (2,3) (north-east) is `None` = the null sentinel.
fn ref_z(i: usize, j: usize) -> Option<f64> {
    if i == 2 && j == 3 {
        None
    } else {
        Some(1000.0 + j as f64 * 100.0 + i as f64)
    }
}

/// IRAP classic: 19 header tokens then column-major, x-fastest, SOUTH row first.
fn irap_fixture() -> String {
    let mut s = String::from(
        "-996 4 100 100\n431000 431200 6521000 6521300\n3 0 431000 6521000\n0 0 0 0 0 0 0\n",
    );
    for j in 0..NROW {
        for i in 0..NCOL {
            let t = match ref_z(i, j) {
                Some(v) => format!("{v} "),
                None => "9999900 ".to_string(),
            };
            s.push_str(&t);
        }
    }
    s.push('\n');
    s
}

/// CPS-3 regular grid (Petrel dialect): `FS*` header, `->` marker, then z values
/// **row-major, first data row = SOUTH (ymin)**, west→east within a row.
fn cps3_fixture() -> String {
    let mut s = String::from(
        "FSASCI 0 1 COMPUTED 0 1.0E+30\n\
         FSATTR 0 0\n\
         FSLIMI 431000 431200 6521000 6521300 1000 1301\n\
         FSNROW 4 3\n\
         FSXINC 100 100\n\
         ->\n",
    );
    for j in 0..NROW {
        for i in 0..NCOL {
            match ref_z(i, j) {
                Some(v) => s.push_str(&format!("{v} ")),
                None => s.push_str("1.0E+30 "),
            }
        }
        s.push('\n');
    }
    s
}

/// EarthVision scattered grid: `#`-comment header then explicit `x y z` per node.
/// Rows are emitted NORTH-first on purpose — EarthVision carries each node's own
/// world coordinates, so ingest order must not affect the result.
fn earthvision_fixture() -> String {
    let mut s = String::from(
        "# Type: scattered data\n\
         # Field: 1 X\n# Field: 2 Y\n# Field: 3 Z\n\
         # Grid_size: 3 x 4\n# Null_value: 1.0e30\n# End:\n",
    );
    for j in (0..NROW).rev() {
        for i in 0..NCOL {
            let (x, y) = (XORI + i as f64 * INC, YORI + j as f64 * INC);
            match ref_z(i, j) {
                Some(v) => s.push_str(&format!("{x} {y} {v}\n")),
                None => s.push_str(&format!("{x} {y} 1.0e30\n")),
            }
        }
    }
    s
}

fn approx(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6
}

/// CPS-3 must ingest to the exact same world-coordinate surface as the IRAP copy
/// of the same asymmetric surface. Fails (structure Y-flipped) before the reader
/// fix; passes after.
#[test]
fn cps3_matches_irap_node_for_node() {
    let d = tmpdir("orient");
    let irap_p = d.join("dome.irap");
    let cps3_p = d.join("dome.cps3grid");
    std::fs::write(&irap_p, irap_fixture()).unwrap();
    std::fs::write(&cps3_p, cps3_fixture()).unwrap();

    let mut geo = GeoData::new(Unit::Metres);
    geo.load_surface("irap", &irap_p).unwrap();
    geo.load_surface("cps3", &cps3_p).unwrap();
    let si = geo.surface("irap").unwrap();
    let sc = geo.surface("cps3").unwrap();

    assert_eq!(
        si.geom, sc.geom,
        "CPS-3 geometry must match the IRAP baseline"
    );
    for j in 0..NROW {
        for i in 0..NCOL {
            let (xi, yi) = si.geom.node_xy(i, j);
            let (xc, yc) = sc.geom.node_xy(i, j);
            assert!(approx(xi, xc) && approx(yi, yc), "node ({i},{j}) world xy");
            let (vi, vc) = (si.values()[[i, j]], sc.values()[[i, j]]);
            match ref_z(i, j) {
                Some(v) => {
                    assert!(approx(vi, v), "IRAP z at ({i},{j})");
                    assert!(approx(vc, v), "CPS-3 z at ({i},{j}) — Y-flipped?");
                }
                None => {
                    assert!(vi.is_nan() && vc.is_nan(), "null at ({i},{j})");
                }
            }
        }
    }
    // The crest sits at the NORTH edge in both: value at (i,3) > value at (i,0).
    assert!(sc.values()[[0, NROW - 1]] > sc.values()[[0, 0]]);
    assert!(approx(sc.geom.node_xy(0, NROW - 1).1, 6521300.0)); // north = ymax
}

/// EarthVision carries explicit per-node coordinates, so it cannot Y-flip; this
/// golden pins that invariant against the same asymmetric surface (feature at the
/// north edge, ingested north-first).
#[test]
fn earthvision_preserves_orientation() {
    let d = tmpdir("orient_ev");
    let ev_p = d.join("dome.earthvisiongrid");
    std::fs::write(&ev_p, earthvision_fixture()).unwrap();

    let mut geo = GeoData::new(Unit::Metres);
    geo.load_points("ev", &ev_p).unwrap();
    let pts = geo.points("ev").unwrap();
    // Grid back onto the reference lattice (nearest = exact at coincident nodes).
    let surf = pts.to_surface(ref_geom(), GridMethod::Nearest).unwrap();
    // South-west node is the surface minimum; north-east the maximum → not flipped.
    assert!(approx(surf.values()[[0, 0]], 1000.0)); // SW, low
    assert!(approx(surf.values()[[1, NROW - 1]], 1301.0)); // north row, defined
    assert!(surf.values()[[0, NROW - 1]] > surf.values()[[0, 0]]);
}
