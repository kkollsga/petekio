//! Gridding — interpolate scattered `(x, y, z)` samples onto a regular
//! `GridGeometry`, producing a `Surface`. Backs [`PointSet::to_surface`].
//! Implements the three `GridMethod` variants; see
//! `dev-docs/designs/gridding-method.md`.

use crate::core::points::{AerialEntry, GridMethod};
use crate::core::surface::Surface;
use crate::foundation::{GeoError, GridGeometry, Result};
use ndarray::Array2;
use rstar::primitives::GeomWithData;
use rstar::RTree;

/// IDW power exponent (`wᵢ = 1/dᵢ^p`); p=2 is the locked default.
const IDW_POWER: f64 = 2.0;

/// Build an areal R*-tree over `coords`' XY, payloaded with the point index.
pub(crate) fn build_rtree(coords: &[[f64; 3]]) -> RTree<AerialEntry> {
    let entries: Vec<AerialEntry> = coords
        .iter()
        .enumerate()
        .map(|(i, c)| GeomWithData::new([c[0], c[1]], i))
        .collect();
    RTree::bulk_load(entries)
}

/// Grid `coords` onto `geom` with `method`. Errors if there are no points.
pub(crate) fn grid(coords: &[[f64; 3]], geom: GridGeometry, method: GridMethod) -> Result<Surface> {
    if coords.is_empty() {
        return Err(GeoError::NotFound(
            "PointSet::to_surface: no points to grid".into(),
        ));
    }
    let values = match method {
        GridMethod::Nearest => grid_nearest(coords, &geom),
        GridMethod::InverseDistance => grid_idw(coords, &geom),
        GridMethod::MinimumCurvature => grid_min_curvature(coords, &geom),
    };
    Surface::new(geom, values)
}

/// Nearest-neighbour: each node takes its areally-closest sample's Z.
fn grid_nearest(coords: &[[f64; 3]], geom: &GridGeometry) -> Array2<f64> {
    let tree = build_rtree(coords);
    let mut out = Array2::from_elem((geom.ncol, geom.nrow), f64::NAN);
    for j in 0..geom.nrow {
        for i in 0..geom.ncol {
            let (x, y) = geom.node_xy(i, j);
            if let Some(e) = tree.nearest_neighbor([x, y]) {
                out[[i, j]] = coords[e.data][2];
            }
        }
    }
    out
}

/// Inverse-distance weighting (global, p=2). Exact at coincident samples.
fn grid_idw(coords: &[[f64; 3]], geom: &GridGeometry) -> Array2<f64> {
    let mut out = Array2::from_elem((geom.ncol, geom.nrow), f64::NAN);
    for j in 0..geom.nrow {
        for i in 0..geom.ncol {
            let (x, y) = geom.node_xy(i, j);
            out[[i, j]] = idw_at(coords, x, y);
        }
    }
    out
}

/// IDW value at a single point.
fn idw_at(coords: &[[f64; 3]], x: f64, y: f64) -> f64 {
    let mut wsum = 0.0;
    let mut vsum = 0.0;
    for c in coords {
        let d2 = (c[0] - x).powi(2) + (c[1] - y).powi(2);
        if d2 == 0.0 {
            return c[2]; // exact hit
        }
        let w = 1.0 / d2.powf(IDW_POWER / 2.0);
        wsum += w;
        vsum += w * c[2];
    }
    if wsum > 0.0 {
        vsum / wsum
    } else {
        f64::NAN
    }
}

/// Briggs minimum-curvature gridding via biharmonic (∇⁴z = 0) SOR relaxation,
/// anchored at the grid nodes nearest each data sample.
///
/// **Scope:** a straightforward, convergent implementation for small/moderate
/// grids — interior nodes use the 13-point biharmonic stencil; near-edge nodes
/// fall back to the 5-point harmonic (Laplacian) update. Data are honoured by
/// snapping each sample to its nearest node and holding those fixed. Linear
/// trends (the exact biharmonic solution) are reproduced to tolerance.
fn grid_min_curvature(coords: &[[f64; 3]], geom: &GridGeometry) -> Array2<f64> {
    let (nc, nr) = (geom.ncol, geom.nrow);
    // Seed with IDW so the relaxation starts from a smooth, in-range field.
    let mut z = grid_idw(coords, geom);

    // Anchor nodes: snap each sample to the nearest node, averaging collisions.
    let mut fixed = Array2::from_elem((nc, nr), false);
    let mut acc: std::collections::HashMap<(usize, usize), (f64, usize)> =
        std::collections::HashMap::new();
    for c in coords {
        if let Some((fi, fj)) = geom.xy_to_ij(c[0], c[1]) {
            let i = fi.round();
            let j = fj.round();
            if i < 0.0 || j < 0.0 {
                continue;
            }
            let (i, j) = (i as usize, j as usize);
            if i < nc && j < nr {
                let e = acc.entry((i, j)).or_insert((0.0, 0));
                e.0 += c[2];
                e.1 += 1;
            }
        }
    }
    for ((i, j), (sum, n)) in acc {
        z[[i, j]] = sum / n as f64;
        fixed[[i, j]] = true;
    }

    if nc < 2 || nr < 2 {
        return z;
    }

    const MAX_ITERS: usize = 5000;
    const TOL: f64 = 1e-6;
    const OMEGA: f64 = 1.5; // SOR over-relaxation

    let range = {
        let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
        for c in coords {
            lo = lo.min(c[2]);
            hi = hi.max(c[2]);
        }
        (hi - lo).abs().max(1.0)
    };

    for _ in 0..MAX_ITERS {
        let mut max_delta = 0.0_f64;
        for j in 0..nr {
            for i in 0..nc {
                if fixed[[i, j]] {
                    continue;
                }
                let target = if i >= 2 && i + 2 < nc && j >= 2 && j + 2 < nr {
                    biharmonic_target(&z, i, j)
                } else {
                    harmonic_target(&z, i, j, nc, nr)
                };
                let old = z[[i, j]];
                let new = old + OMEGA * (target - old);
                z[[i, j]] = new;
                max_delta = max_delta.max((new - old).abs());
            }
        }
        if max_delta < TOL * range {
            break;
        }
    }
    z
}

/// 13-point biharmonic stencil solved for the centre node (∇⁴z = 0):
/// `z = [8·(N+S+E+W) − 2·(NE+NW+SE+SW) − (NN+SS+EE+WW)] / 20`.
fn biharmonic_target(z: &Array2<f64>, i: usize, j: usize) -> f64 {
    let n = z[[i, j + 1]];
    let s = z[[i, j - 1]];
    let e = z[[i + 1, j]];
    let w = z[[i - 1, j]];
    let ne = z[[i + 1, j + 1]];
    let nw = z[[i - 1, j + 1]];
    let se = z[[i + 1, j - 1]];
    let sw = z[[i - 1, j - 1]];
    let nn = z[[i, j + 2]];
    let ss = z[[i, j - 2]];
    let ee = z[[i + 2, j]];
    let ww = z[[i - 2, j]];
    (8.0 * (n + s + e + w) - 2.0 * (ne + nw + se + sw) - (nn + ss + ee + ww)) / 20.0
}

/// 5-point harmonic (Laplacian) update, averaging available orthogonal
/// neighbours — used for near-edge nodes where the biharmonic stencil overruns.
fn harmonic_target(z: &Array2<f64>, i: usize, j: usize, nc: usize, nr: usize) -> f64 {
    let mut sum = 0.0;
    let mut n = 0.0;
    if i > 0 {
        sum += z[[i - 1, j]];
        n += 1.0;
    }
    if i + 1 < nc {
        sum += z[[i + 1, j]];
        n += 1.0;
    }
    if j > 0 {
        sum += z[[i, j - 1]];
        n += 1.0;
    }
    if j + 1 < nr {
        sum += z[[i, j + 1]];
        n += 1.0;
    }
    if n > 0.0 {
        sum / n
    } else {
        z[[i, j]]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    fn geom(ncol: usize, nrow: usize) -> GridGeometry {
        GridGeometry {
            xori: 0.0,
            yori: 0.0,
            xinc: 1.0,
            yinc: 1.0,
            ncol,
            nrow,
            rotation_deg: 0.0,
            yflip: false,
        }
    }

    #[test]
    fn nearest_hand_calc() {
        // three corners of a 3×3 unit grid
        let coords = vec![[0.0, 0.0, 1.0], [2.0, 0.0, 2.0], [0.0, 2.0, 3.0]];
        let s = grid(&coords, geom(3, 3), GridMethod::Nearest).unwrap();
        let v = s.values();
        assert_relative_eq!(v[[0, 0]], 1.0); // at (0,0) sample
        assert_relative_eq!(v[[2, 0]], 2.0); // at (2,0) sample
        assert_relative_eq!(v[[0, 2]], 3.0); // at (0,2) sample
                                             // node (2,2)=(2,2) is equidistant-ish; nearest is (2,0)->2 or (0,2)->3
                                             // dist to (2,0): 2 ; to (0,2): 2 ; to (0,0): 2.83 -> tie broken by tree
        let corner = v[[2, 2]];
        assert!(corner == 2.0 || corner == 3.0);
    }

    #[test]
    fn idw_exact_at_samples_and_midpoint() {
        let coords = vec![[0.0, 0.0, 0.0], [2.0, 0.0, 10.0]];
        let s = grid(&coords, geom(3, 1), GridMethod::InverseDistance).unwrap();
        let v = s.values();
        assert_relative_eq!(v[[0, 0]], 0.0); // on sample A
        assert_relative_eq!(v[[2, 0]], 10.0); // on sample B
                                              // midpoint (1,0): equal distance -> simple average = 5.0
        assert_relative_eq!(v[[1, 0]], 5.0);
    }

    #[test]
    fn idw_weighting_hand_calc() {
        // node at x=1: dA=1 (z=0), dB=3 (z=12). w=1/d^2 -> wA=1, wB=1/9.
        // z = (1*0 + (1/9)*12)/(1+1/9) = (12/9)/(10/9) = 1.2
        let coords = vec![[0.0, 0.0, 0.0], [4.0, 0.0, 12.0]];
        let g = GridGeometry {
            xori: 1.0,
            yori: 0.0,
            xinc: 1.0,
            yinc: 1.0,
            ncol: 1,
            nrow: 1,
            rotation_deg: 0.0,
            yflip: false,
        };
        let s = grid(&coords, g, GridMethod::InverseDistance).unwrap();
        assert_relative_eq!(s.values()[[0, 0]], 1.2, epsilon = 1e-12);
    }

    #[test]
    fn min_curvature_reproduces_plane() {
        // A linear trend `z = 2x + 3y + 1` is the exact biharmonic (∇⁴z=0)
        // solution. Anchoring the full boundary ring (Dirichlet BC) makes that
        // solution unique, so the SOR relaxation must converge to the plane at
        // every free interior node. This is the convergence smoke test.
        let plane = |x: f64, y: f64| 2.0 * x + 3.0 * y + 1.0;
        let g = geom(7, 7);
        let mut coords = Vec::new();
        for j in 0..g.nrow {
            for i in 0..g.ncol {
                if i == 0 || j == 0 || i == g.ncol - 1 || j == g.nrow - 1 {
                    let (x, y) = g.node_xy(i, j);
                    coords.push([x, y, plane(x, y)]);
                }
            }
        }
        let s = grid(&coords, g.clone(), GridMethod::MinimumCurvature).unwrap();
        let v = s.values();
        for j in 1..g.nrow - 1 {
            for i in 1..g.ncol - 1 {
                let (x, y) = g.node_xy(i, j);
                assert_relative_eq!(v[[i, j]], plane(x, y), epsilon = 1e-3);
            }
        }
    }

    #[test]
    fn empty_points_error() {
        assert!(grid(&[], geom(2, 2), GridMethod::Nearest).is_err());
    }
}
