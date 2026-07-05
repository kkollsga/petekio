//! Surface filtering and outline ops: NaN-aware smoothing and the boundary
//! polygon of the defined region. Like the arithmetic ops, these return **new**
//! values and respect the `NaN` (undefined) convention.

use super::{PolygonSet, Surface};
use ndarray::Array2;

impl Surface {
    /// NaN-aware moving-average smoothing with a Chebyshev (square) window of the
    /// given `radius`. Each **defined** node is replaced by the mean of the
    /// defined nodes in its `(2·radius+1)²` window; undefined (`NaN`) nodes stay
    /// undefined, so the defined mask is preserved (smoothing never grows the
    /// surface). `radius == 0` is the identity. Returns a new surface.
    pub fn smooth(&self, radius: usize) -> Surface {
        if radius == 0 {
            return Surface::from_values_unchecked(self.geom.clone(), self.values().clone());
        }
        let (nc, nr) = (self.geom.ncol, self.geom.nrow);
        let v = self.values();
        let r = radius as isize;
        let mut out = Array2::from_elem((nc, nr), f64::NAN);
        for j in 0..nr {
            for i in 0..nc {
                if v[[i, j]].is_nan() {
                    continue; // preserve the undefined mask
                }
                let mut sum = 0.0;
                let mut count = 0usize;
                for dj in -r..=r {
                    for di in -r..=r {
                        let ii = i as isize + di;
                        let jj = j as isize + dj;
                        if ii < 0 || jj < 0 || ii >= nc as isize || jj >= nr as isize {
                            continue;
                        }
                        let val = v[[ii as usize, jj as usize]];
                        if !val.is_nan() {
                            sum += val;
                            count += 1;
                        }
                    }
                }
                if count > 0 {
                    out[[i, j]] = sum / count as f64;
                }
            }
        }
        Surface::from_values_unchecked(self.geom.clone(), out)
    }

    /// The boundary polygon enclosing the surface's defined region — the convex
    /// hull (in world XY) of all non-`NaN` nodes. Returns `None` if fewer than
    /// three defined nodes exist.
    ///
    /// This is a **convex** outline (a drainage-boundary approximation); a
    /// concave hull is a future refinement. Backs `ModelInputs::boundary` when no
    /// explicit boundary polygon is supplied.
    pub fn boundary_polygon(&self) -> Option<PolygonSet> {
        let (nc, nr) = (self.geom.ncol, self.geom.nrow);
        let v = self.values();
        let mut pts: Vec<[f64; 2]> = Vec::new();
        for j in 0..nr {
            for i in 0..nc {
                if !v[[i, j]].is_nan() {
                    let (x, y) = self.geom.node_xy(i, j);
                    pts.push([x, y]);
                }
            }
        }
        if pts.len() < 3 {
            return None;
        }
        let hull = convex_hull(pts);
        if hull.len() < 3 {
            return None;
        }
        let ring: Vec<[f64; 3]> = hull.iter().map(|p| [p[0], p[1], 0.0]).collect();
        Some(PolygonSet::from_rings(vec![ring]))
    }
}

/// Andrew's monotone-chain convex hull over XY points; returns the hull vertices
/// counter-clockwise (no repeated closing vertex). Degenerate (collinear) inputs
/// may return fewer than three points.
fn convex_hull(mut pts: Vec<[f64; 2]>) -> Vec<[f64; 2]> {
    pts.sort_by(|a, b| a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1])));
    pts.dedup();
    if pts.len() < 3 {
        return pts;
    }
    let cross = |o: [f64; 2], a: [f64; 2], b: [f64; 2]| {
        (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0])
    };
    let mut lower: Vec<[f64; 2]> = Vec::new();
    for &p in &pts {
        while lower.len() >= 2 && cross(lower[lower.len() - 2], lower[lower.len() - 1], p) <= 0.0 {
            lower.pop();
        }
        lower.push(p);
    }
    let mut upper: Vec<[f64; 2]> = Vec::new();
    for &p in pts.iter().rev() {
        while upper.len() >= 2 && cross(upper[upper.len() - 2], upper[upper.len() - 1], p) <= 0.0 {
            upper.pop();
        }
        upper.push(p);
    }
    lower.pop();
    upper.pop();
    lower.extend(upper);
    lower
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::foundation::GridGeometry;
    use approx::assert_relative_eq;

    fn geom(n: usize, inc: f64) -> GridGeometry {
        GridGeometry {
            xori: 0.0,
            yori: 0.0,
            xinc: inc,
            yinc: inc,
            ncol: n,
            nrow: n,
            rotation_deg: 0.0,
            yflip: false,
        }
    }

    #[test]
    fn smooth_leaves_constant_field_unchanged() {
        let s = Surface::constant(geom(5, 10.0), 7.0);
        let out = s.smooth(1);
        for v in out.values().iter() {
            assert_relative_eq!(*v, 7.0);
        }
    }

    #[test]
    fn smooth_reduces_a_spike() {
        let mut v = ndarray::Array2::from_elem((3, 3), 0.0);
        v[[1, 1]] = 9.0; // central spike
        let s = Surface::new(geom(3, 10.0), v).unwrap();
        let out = s.smooth(1);
        // Centre averages the 3×3 window (one 9, eight 0s) → 1.0; spike flattened.
        assert_relative_eq!(out.values()[[1, 1]], 1.0);
        assert!(out.values()[[1, 1]] < 9.0);
    }

    #[test]
    fn smooth_preserves_nan_mask() {
        let mut v = ndarray::Array2::from_elem((3, 3), 1.0);
        v[[0, 0]] = f64::NAN;
        let out = Surface::new(geom(3, 10.0), v).unwrap().smooth(1);
        assert!(out.values()[[0, 0]].is_nan()); // undefined centre stays undefined
        assert_relative_eq!(out.values()[[2, 2]], 1.0);
    }

    #[test]
    fn boundary_polygon_hull_area() {
        // 3×3 fully-defined grid, nodes at 0/10/20 → hull is the 20×20 square.
        let s = Surface::constant(geom(3, 10.0), 1.0);
        let poly = s.boundary_polygon().unwrap();
        assert_relative_eq!(poly.area(), 400.0, epsilon = 1e-9);
    }

    #[test]
    fn boundary_polygon_none_when_too_few_defined() {
        let mut v = ndarray::Array2::from_elem((3, 3), f64::NAN);
        v[[0, 0]] = 1.0;
        v[[1, 1]] = 1.0; // only two defined nodes
        let s = Surface::new(geom(3, 10.0), v).unwrap();
        assert!(s.boundary_polygon().is_none());
    }
}
