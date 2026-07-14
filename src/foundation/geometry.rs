//! Geometry primitives: `Point3`, `BBox`, and the rotatable `GridGeometry`
//! lattice (the IRAP/RMS model) with its forward/inverse coordinate maps.

/// A point in project coordinates: x = Easting, y = Northing, and z =
/// **negative-down elevation** (subsea; directly comparable with a surface).
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
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
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
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

    /// `true` when the lattice is axis-aligned (`rotation_deg == 0`).
    pub fn is_axis_aligned(&self) -> bool {
        self.rotation_deg == 0.0
    }

    /// Map onto petekTools' field-for-field-identical `Lattice` — the seam to
    /// the shared gridding / resample kernels. One home for the conversion.
    pub(crate) fn to_lattice(&self) -> petektools::Lattice {
        petektools::Lattice {
            xori: self.xori,
            yori: self.yori,
            xinc: self.xinc,
            yinc: self.yinc,
            ncol: self.ncol,
            nrow: self.nrow,
            rotation_deg: self.rotation_deg,
            yflip: self.yflip,
        }
    }

    /// World `(x, y)` of node `(i, j)`. `node_xy(0, 0) == (xori, yori)`.
    pub fn node_xy(&self, i: usize, j: usize) -> (f64, f64) {
        self.to_lattice().node_xy(i, j)
    }

    /// Fractional node coordinates `(fi, fj)` for world `(x, y)` — the inverse
    /// of [`node_xy`](Self::node_xy). `None` for a degenerate (zero-spacing)
    /// geometry. The result may lie outside `[0, ncol-1] × [0, nrow-1]`.
    pub fn xy_to_ij(&self, x: f64, y: f64) -> Option<(f64, f64)> {
        self.to_lattice().xy_to_ij(x, y)
    }

    /// Axis-aligned bounding box of all nodes.
    pub fn bbox(&self) -> BBox {
        let bbox = self.to_lattice().bbox();
        BBox {
            xmin: bbox.xmin,
            ymin: bbox.ymin,
            xmax: bbox.xmax,
            ymax: bbox.ymax,
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
    fn fractional_world_cell_roundtrip_matches_tools_lattice() {
        let g = geom(30.0, true);
        let lattice = g.to_lattice();
        for &(i, j) in &[(0, 0), (2, 0), (0, 3), (1, 2)] {
            assert_eq!(g.node_xy(i, j), lattice.node_xy(i, j));
        }
        let (origin_x, origin_y) = g.node_xy(0, 0);
        let (i_x, i_y) = g.node_xy(1, 0);
        let (j_x, j_y) = g.node_xy(0, 1);
        let (fi, fj) = (1.25, 2.5);
        let world = (
            origin_x + fi * (i_x - origin_x) + fj * (j_x - origin_x),
            origin_y + fi * (i_y - origin_y) + fj * (j_y - origin_y),
        );
        let own = g.xy_to_ij(world.0, world.1).unwrap();
        let tools = lattice.xy_to_ij(world.0, world.1).unwrap();
        assert_eq!(own, tools);
        assert_relative_eq!(own.0, fi, epsilon = 1e-10);
        assert_relative_eq!(own.1, fj, epsilon = 1e-10);
        assert_eq!(g.bbox().xmin, lattice.bbox().xmin);
        assert_eq!(g.bbox().ymax, lattice.bbox().ymax);
    }

    #[test]
    fn zero_rotation_delegation_is_bit_compatible() {
        let g = geom(0.0, false);
        assert_eq!(g.node_xy(2, 3), (1100.0, 2075.0));
        assert_eq!(g.xy_to_ij(1025.0, 2037.5), Some((0.5, 1.5)));
    }

    #[test]
    fn axis_aligned_node_positions() {
        let g = geom(0.0, false);
        assert_relative_eq!(g.node_xy(2, 0).0, 1100.0); // 1000 + 2*50
        assert_relative_eq!(g.node_xy(0, 3).1, 2075.0); // 2000 + 3*25
    }
}
