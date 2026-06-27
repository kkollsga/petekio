//! Geometry primitives: `Point3`, `BBox`, and the rotatable `GridGeometry`
//! lattice (the IRAP/RMS model) with its forward/inverse coordinate maps.

/// A point in project coordinates (x = Easting, y = Northing, z = depth,
/// increasing **downward**).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl Point3 {
    pub fn new(x: f64, y: f64, z: f64) -> Self {
        Self { x, y, z }
    }
}

/// An axis-aligned 2-D bounding box.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BBox {
    pub xmin: f64,
    pub ymin: f64,
    pub xmax: f64,
    pub ymax: f64,
}

/// A regular, rotatable areal lattice (IRAP/RMS model). Node `(i, j)` runs
/// `i` along the column/x axis (`ncol` nodes) and `j` along the row/y axis
/// (`nrow` nodes).
#[derive(Debug, Clone, PartialEq)]
pub struct GridGeometry {
    /// Origin x (node 0,0).
    pub xori: f64,
    /// Origin y (node 0,0).
    pub yori: f64,
    /// Node spacing along the column/x axis.
    pub xinc: f64,
    /// Node spacing along the row/y axis.
    pub yinc: f64,
    /// Node count along x.
    pub ncol: usize,
    /// Node count along y.
    pub nrow: usize,
    /// Rotation in degrees, counter-clockwise of the I-axis from East.
    pub rotation_deg: f64,
    /// If true, the row/y axis is flipped (origin becomes the upper-left
    /// corner; y decreases along the row axis).
    pub yflip: bool,
}

impl GridGeometry {
    /// `+1.0` normally, `-1.0` when `yflip` is set.
    pub fn yflip_factor(&self) -> f64 {
        if self.yflip {
            -1.0
        } else {
            1.0
        }
    }

    /// World `(x, y)` of node `(i, j)`. `node_xy(0, 0) == (xori, yori)`.
    pub fn node_xy(&self, i: usize, j: usize) -> (f64, f64) {
        let (s, c) = self.rotation_deg.to_radians().sin_cos();
        let di = i as f64 * self.xinc;
        let dj = j as f64 * self.yinc * self.yflip_factor();
        (self.xori + di * c - dj * s, self.yori + di * s + dj * c)
    }

    /// Fractional node coordinates `(fi, fj)` for world `(x, y)` — the inverse
    /// of [`node_xy`](Self::node_xy). `None` for a degenerate (zero-spacing)
    /// geometry. The result may lie outside `[0, ncol-1] × [0, nrow-1]`.
    pub fn xy_to_ij(&self, x: f64, y: f64) -> Option<(f64, f64)> {
        if self.xinc == 0.0 || self.yinc == 0.0 {
            return None;
        }
        let (s, c) = self.rotation_deg.to_radians().sin_cos();
        let dx = x - self.xori;
        let dy = y - self.yori;
        let u = dx * c + dy * s; // along x axis  = i * xinc
        let v = -dx * s + dy * c; // along y axis = j * yinc * yflip
        Some((u / self.xinc, v / (self.yinc * self.yflip_factor())))
    }

    /// Axis-aligned bounding box of all nodes.
    pub fn bbox(&self) -> BBox {
        let ni = self.ncol.saturating_sub(1);
        let nj = self.nrow.saturating_sub(1);
        let corners = [
            self.node_xy(0, 0),
            self.node_xy(ni, 0),
            self.node_xy(0, nj),
            self.node_xy(ni, nj),
        ];
        let xmin = corners.iter().map(|p| p.0).fold(f64::INFINITY, f64::min);
        let xmax = corners
            .iter()
            .map(|p| p.0)
            .fold(f64::NEG_INFINITY, f64::max);
        let ymin = corners.iter().map(|p| p.1).fold(f64::INFINITY, f64::min);
        let ymax = corners
            .iter()
            .map(|p| p.1)
            .fold(f64::NEG_INFINITY, f64::max);
        BBox {
            xmin,
            ymin,
            xmax,
            ymax,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    fn geom(rotation_deg: f64, yflip: bool) -> GridGeometry {
        GridGeometry {
            xori: 1000.0,
            yori: 2000.0,
            xinc: 50.0,
            yinc: 25.0,
            ncol: 3,
            nrow: 4,
            rotation_deg,
            yflip,
        }
    }

    #[test]
    fn origin_node_is_origin() {
        let g = geom(30.0, false);
        let (x, y) = g.node_xy(0, 0);
        assert_relative_eq!(x, 1000.0);
        assert_relative_eq!(y, 2000.0);
    }

    #[test]
    fn node_xy_inverse_roundtrip_rotated() {
        let g = geom(30.0, false);
        for &(i, j) in &[(0, 0), (2, 0), (0, 3), (2, 3), (1, 2)] {
            let (x, y) = g.node_xy(i, j);
            let (fi, fj) = g.xy_to_ij(x, y).unwrap();
            assert_relative_eq!(fi, i as f64, epsilon = 1e-9);
            assert_relative_eq!(fj, j as f64, epsilon = 1e-9);
        }
    }

    #[test]
    fn node_xy_inverse_roundtrip_yflip() {
        let g = geom(15.0, true);
        for &(i, j) in &[(0, 0), (2, 0), (0, 3), (1, 2)] {
            let (x, y) = g.node_xy(i, j);
            let (fi, fj) = g.xy_to_ij(x, y).unwrap();
            assert_relative_eq!(fi, i as f64, epsilon = 1e-9);
            assert_relative_eq!(fj, j as f64, epsilon = 1e-9);
        }
    }

    #[test]
    fn axis_aligned_node_positions() {
        let g = geom(0.0, false);
        assert_relative_eq!(g.node_xy(2, 0).0, 1100.0); // 1000 + 2*50
        assert_relative_eq!(g.node_xy(0, 3).1, 2075.0); // 2000 + 3*25
    }
}
