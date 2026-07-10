//! `StructuredShell` — the level-2 geometry shell.
//!
//! `(i, j)`-organized nodes with **explicit per-node XY** (two `(ncol, nrow)`
//! arrays): the exact home for Petrel/EarthVision exports whose nodes are
//! fault-shifted or curvilinear and therefore lie on no single
//! [`GridGeometry`]. Purely topological/positional — never a function of z.
//! `nominal_geometry` is metadata, never the coordinates. Immutable once
//! built; share via `Arc`.

use super::fit::fit_grid_from_indexed;
use super::mesh::{signed_area2, MeshShell, WalkLabel};
use crate::core::PolygonSet;
use crate::foundation::{BBox, GeoError, GridGeometry, Result};
use ndarray::Array2;

/// A logically regular shell with explicit per-node coordinates (level 2).
#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(try_from = "StructuredShellData")]
pub struct StructuredShell {
    ncol: usize,
    nrow: usize,
    x: Array2<f64>,
    y: Array2<f64>,
    nominal_geometry: Option<GridGeometry>,
    edge: PolygonSet,
}

/// Serialized shape of a [`StructuredShell`]; deserialization routes through
/// [`StructuredShell::new`] so a decoded shell is always validated.
#[derive(serde::Serialize, serde::Deserialize)]
struct StructuredShellData {
    ncol: usize,
    nrow: usize,
    x: Array2<f64>,
    y: Array2<f64>,
    nominal_geometry: Option<GridGeometry>,
    edge: PolygonSet,
}

impl TryFrom<StructuredShellData> for StructuredShell {
    type Error = GeoError;
    fn try_from(d: StructuredShellData) -> Result<StructuredShell> {
        let shell = StructuredShell::new(d.x, d.y, d.nominal_geometry, d.edge)?;
        if (shell.ncol, shell.nrow) != (d.ncol, d.nrow) {
            return Err(GeoError::GeometryMismatch(
                "StructuredShell payload ncol/nrow disagree with the coordinate arrays".into(),
            ));
        }
        Ok(shell)
    }
}

impl StructuredShell {
    /// Build a shell from explicit node coordinate arrays, both shaped
    /// `(ncol, nrow)` and non-empty.
    pub fn new(
        x: Array2<f64>,
        y: Array2<f64>,
        nominal_geometry: Option<GridGeometry>,
        edge: PolygonSet,
    ) -> Result<StructuredShell> {
        let shape = x.dim();
        if shape.0 == 0 || shape.1 == 0 {
            return Err(GeoError::GeometryMismatch(
                "StructuredShell::new: shape must be non-empty".into(),
            ));
        }
        if y.dim() != shape {
            return Err(GeoError::GeometryMismatch(format!(
                "StructuredShell::new: x/y shapes differ: x={:?}, y={:?}",
                x.dim(),
                y.dim()
            )));
        }
        Ok(StructuredShell {
            ncol: shape.0,
            nrow: shape.1,
            x,
            y,
            nominal_geometry,
            edge,
        })
    }

    pub fn ncol(&self) -> usize {
        self.ncol
    }

    pub fn nrow(&self) -> usize {
        self.nrow
    }

    /// X node coordinates, shape `(ncol, nrow)`.
    pub fn x(&self) -> &Array2<f64> {
        &self.x
    }

    /// Y node coordinates, shape `(ncol, nrow)`.
    pub fn y(&self) -> &Array2<f64> {
        &self.y
    }

    /// Optional approximate regular geometry. Metadata only; consumers must
    /// not treat it as the canonical node coordinate model.
    pub fn nominal_geometry(&self) -> Option<&GridGeometry> {
        self.nominal_geometry.as_ref()
    }

    /// Edge polygon in modelling coordinates.
    pub fn edge(&self) -> &PolygonSet {
        &self.edge
    }

    /// World `(x, y)` of logical node `(i, j)`.
    pub fn node_xy(&self, i: usize, j: usize) -> Result<(f64, f64)> {
        self.check_node(i, j)?;
        Ok((self.x[[i, j]], self.y[[i, j]]))
    }

    pub(crate) fn check_node(&self, i: usize, j: usize) -> Result<()> {
        if i >= self.ncol || j >= self.nrow {
            return Err(GeoError::OutOfRange(format!(
                "structured surface node ({i}, {j}) outside shape (ncol={}, nrow={})",
                self.ncol, self.nrow
            )));
        }
        Ok(())
    }

    /// Axis-aligned bounding box over finite XY nodes.
    pub fn bbox(&self) -> BBox {
        let mut b = BBox {
            xmin: f64::INFINITY,
            ymin: f64::INFINITY,
            xmax: f64::NEG_INFINITY,
            ymax: f64::NEG_INFINITY,
        };
        let mut any = false;
        for (x, y) in self.x.iter().zip(self.y.iter()) {
            if x.is_finite() && y.is_finite() {
                any = true;
                b.xmin = b.xmin.min(*x);
                b.xmax = b.xmax.max(*x);
                b.ymin = b.ymin.min(*y);
                b.ymax = b.ymax.max(*y);
            }
        }
        if any {
            b
        } else {
            BBox {
                xmin: f64::NAN,
                ymin: f64::NAN,
                xmax: f64::NAN,
                ymax: f64::NAN,
            }
        }
    }

    /// Explode into a [`MeshShell`] (the free upward conversion): every
    /// finite-XY node becomes a mesh node labelled `(0, i, j)`, and every cell
    /// whose four corners are present quad-splits along the consistent
    /// `(i, j)`–`(i+1, j+1)` diagonal into two CCW triangles. Node identity is
    /// preserved on the labels. Errors when no complete cell exists.
    pub fn to_mesh_shell(&self) -> Result<MeshShell> {
        let mut id = Array2::from_elem((self.ncol, self.nrow), u32::MAX);
        let mut nodes: Vec<[f64; 2]> = Vec::new();
        let mut labels: Vec<Option<WalkLabel>> = Vec::new();
        for j in 0..self.nrow {
            for i in 0..self.ncol {
                let (x, y) = (self.x[[i, j]], self.y[[i, j]]);
                if x.is_finite() && y.is_finite() {
                    id[[i, j]] = nodes.len() as u32;
                    nodes.push([x, y]);
                    labels.push(Some((0, i as i32, j as i32)));
                }
            }
        }

        let mut triangles: Vec<[u32; 3]> = Vec::new();
        for j in 0..self.nrow.saturating_sub(1) {
            for i in 0..self.ncol.saturating_sub(1) {
                let (n00, n10, n01, n11) = (
                    id[[i, j]],
                    id[[i + 1, j]],
                    id[[i, j + 1]],
                    id[[i + 1, j + 1]],
                );
                if n00 == u32::MAX || n10 == u32::MAX || n01 == u32::MAX || n11 == u32::MAX {
                    continue;
                }
                // Consistent diagonal: (i, j) – (i+1, j+1).
                push_ccw(&mut triangles, &nodes, [n00, n10, n11]);
                push_ccw(&mut triangles, &nodes, [n00, n11, n01]);
            }
        }
        if triangles.is_empty() {
            return Err(GeoError::GeometryInference(
                "structured shell has no complete cell to triangulate".into(),
            ));
        }
        MeshShell::from_triangles(nodes, triangles, labels)
    }

    /// Fit a regular [`GridGeometry`] to the shell's `(i, j)`-indexed nodes
    /// (the lossy downward conversion). Errors when the mesh is curvilinear —
    /// i.e. when the nodes do not sit on any single regular lattice within
    /// `tolerance`.
    pub fn infer_grid(&self, tolerance: f64) -> Result<GridGeometry> {
        let mut indexed = Vec::new();
        for j in 0..self.nrow {
            for i in 0..self.ncol {
                let (x, y) = (self.x[[i, j]], self.y[[i, j]]);
                if x.is_finite() && y.is_finite() {
                    indexed.push((i as isize, j as isize, x, y));
                }
            }
        }
        fit_grid_from_indexed(&indexed, tolerance)
    }
}

impl std::fmt::Debug for StructuredShell {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StructuredShell")
            .field("ncol", &self.ncol)
            .field("nrow", &self.nrow)
            .finish()
    }
}

/// Push a triangle, flipping the winding when it is clockwise in world XY.
fn push_ccw(triangles: &mut Vec<[u32; 3]>, nodes: &[[f64; 2]], t: [u32; 3]) {
    let (a, b, c) = (
        nodes[t[0] as usize],
        nodes[t[1] as usize],
        nodes[t[2] as usize],
    );
    if signed_area2(a, b, c) < 0.0 {
        triangles.push([t[0], t[2], t[1]]);
    } else {
        triangles.push(t);
    }
}

impl GridGeometry {
    /// Explode the rigid grid into a [`StructuredShell`] (the free upward
    /// conversion): per-node XY computed from the lattice, `nominal_geometry`
    /// set to this grid, edge = the full rectangular footprint.
    pub fn to_structured_shell(&self) -> StructuredShell {
        let mut x = Array2::zeros((self.ncol, self.nrow));
        let mut y = Array2::zeros((self.ncol, self.nrow));
        for j in 0..self.nrow {
            for i in 0..self.ncol {
                let (nx, ny) = self.node_xy(i, j);
                x[[i, j]] = nx;
                y[[i, j]] = ny;
            }
        }
        let edge = PolygonSet::from_grid_geometry(self);
        StructuredShell::new(x, y, Some(self.clone()), edge)
            .expect("a GridGeometry always yields matching non-empty arrays")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    fn grid() -> GridGeometry {
        GridGeometry {
            xori: 1000.0,
            yori: 2000.0,
            xinc: 50.0,
            yinc: 25.0,
            ncol: 4,
            nrow: 3,
            rotation_deg: 30.0,
            yflip: false,
        }
    }

    #[test]
    fn grid_to_structured_shell_reproduces_node_xy() {
        let g = grid();
        let shell = g.to_structured_shell();
        assert_eq!((shell.ncol(), shell.nrow()), (g.ncol, g.nrow));
        for j in 0..g.nrow {
            for i in 0..g.ncol {
                let (gx, gy) = g.node_xy(i, j);
                let (sx, sy) = shell.node_xy(i, j).unwrap();
                assert_eq!((gx, gy), (sx, sy));
            }
        }
        assert_eq!(shell.nominal_geometry(), Some(&g));
    }

    #[test]
    fn structured_infer_grid_round_trips() {
        let g = grid();
        let back = g.to_structured_shell().infer_grid(1e-6).unwrap();
        assert_relative_eq!(back.xori, g.xori, epsilon = 1e-6);
        assert_relative_eq!(back.yori, g.yori, epsilon = 1e-6);
        assert_relative_eq!(back.xinc, g.xinc, epsilon = 1e-9);
        assert_relative_eq!(back.yinc, g.yinc, epsilon = 1e-9);
        assert_relative_eq!(back.rotation_deg, g.rotation_deg, epsilon = 1e-9);
        assert_eq!((back.ncol, back.nrow), (g.ncol, g.nrow));
    }

    #[test]
    fn infer_grid_refuses_a_curvilinear_shell() {
        // Node XY swell with i — no single lattice describes them.
        let (ncol, nrow) = (6usize, 5usize);
        let mut x = Array2::zeros((ncol, nrow));
        let mut y = Array2::zeros((ncol, nrow));
        for j in 0..nrow {
            for i in 0..ncol {
                let swell = 1.0 + 0.15 * i as f64;
                x[[i, j]] = 50.0 * i as f64 * swell;
                y[[i, j]] = 50.0 * j as f64 * (1.0 + 0.1 * j as f64);
            }
        }
        let edge = PolygonSet::convex_hull_xy(
            x.iter()
                .zip(y.iter())
                .map(|(x, y)| [*x, *y])
                .collect::<Vec<_>>(),
        )
        .unwrap();
        let shell = StructuredShell::new(x, y, None, edge).unwrap();
        assert!(shell.infer_grid(1e-3).is_err());
        // But the upward path still works: it does not need regularity.
        let mesh = shell.to_mesh_shell().unwrap();
        assert_eq!(mesh.n_nodes(), ncol * nrow);
        assert_eq!(mesh.n_triangles(), 2 * (ncol - 1) * (nrow - 1));
    }

    #[test]
    fn to_mesh_shell_skips_incomplete_cells_and_keeps_labels() {
        let g = GridGeometry {
            xori: 0.0,
            yori: 0.0,
            xinc: 10.0,
            yinc: 10.0,
            ncol: 3,
            nrow: 3,
            rotation_deg: 0.0,
            yflip: false,
        };
        let shell0 = g.to_structured_shell();
        let mut x = shell0.x().clone();
        let mut y = shell0.y().clone();
        // Knock out the corner node (2, 2): its two incident cells lose one
        // corner each; only that cell (1,1)...(2,2) is incomplete.
        x[[2, 2]] = f64::NAN;
        y[[2, 2]] = f64::NAN;
        let shell = StructuredShell::new(x, y, Some(g), shell0.edge().clone()).unwrap();
        let mesh = shell.to_mesh_shell().unwrap();
        assert_eq!(mesh.n_nodes(), 8);
        assert_eq!(mesh.n_triangles(), 2 * 3); // 4 cells - 1 incomplete
        for (k, lab) in mesh.labels().iter().enumerate() {
            let (b, i, j) = lab.expect("every structured node is labelled");
            assert_eq!(b, 0);
            let (nx, ny) = shell.node_xy(i as usize, j as usize).unwrap();
            assert_eq!([nx, ny], mesh.nodes()[k]);
        }
    }
}
