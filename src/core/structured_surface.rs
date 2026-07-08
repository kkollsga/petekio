//! `StructuredMeshSurface` — a logically regular surface whose nodes carry
//! explicit XY coordinates.
//!
//! This is the Petrel/fault-shifted surface home: the surface has a rectangular
//! `(column, row)` topology, but it does **not** claim that all nodes lie on one
//! global affine `GridGeometry`. Regular gridded surfaces stay in [`Surface`].

use crate::core::PolygonSet;
use crate::foundation::{
    BBox, GeoError, GridGeometry, HasHistory, OperationHistory, Result, Stats,
};
use ndarray::Array2;

/// A logically regular surface with explicit per-node coordinates.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct StructuredMeshSurface {
    ncol: usize,
    nrow: usize,
    x: Array2<f64>,
    y: Array2<f64>,
    values: Array2<f64>,
    nominal_geometry: Option<GridGeometry>,
    edge: PolygonSet,
    #[serde(default)]
    history: OperationHistory,
}

impl StructuredMeshSurface {
    /// Build a structured mesh surface from explicit node coordinate/value
    /// arrays. Arrays must all be shaped `(ncol, nrow)`.
    pub fn new(
        x: Array2<f64>,
        y: Array2<f64>,
        values: Array2<f64>,
        nominal_geometry: Option<GridGeometry>,
        edge: PolygonSet,
    ) -> Result<Self> {
        let shape = x.dim();
        if shape.0 == 0 || shape.1 == 0 {
            return Err(GeoError::GeometryMismatch(
                "StructuredMeshSurface::new: shape must be non-empty".into(),
            ));
        }
        if y.dim() != shape || values.dim() != shape {
            return Err(GeoError::GeometryMismatch(format!(
                "StructuredMeshSurface::new: x/y/values shapes differ: x={:?}, y={:?}, values={:?}",
                x.dim(),
                y.dim(),
                values.dim()
            )));
        }
        Ok(Self {
            ncol: shape.0,
            nrow: shape.1,
            x,
            y,
            values,
            nominal_geometry,
            edge,
            history: OperationHistory::from_entry("structured_surface.new"),
        })
    }

    /// Stable kind label for dispatch/reporting.
    pub fn kind(&self) -> &'static str {
        "structured_mesh"
    }

    pub fn ncol(&self) -> usize {
        self.ncol
    }

    pub fn nrow(&self) -> usize {
        self.nrow
    }

    pub fn x(&self) -> &Array2<f64> {
        &self.x
    }

    pub fn y(&self) -> &Array2<f64> {
        &self.y
    }

    pub fn values(&self) -> &Array2<f64> {
        &self.values
    }

    /// Optional approximate regular geometry. This is metadata only; consumers
    /// must not treat it as the canonical node coordinate model.
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

    /// Primary value at logical node `(i, j)`.
    pub fn z(&self, i: usize, j: usize) -> Result<f64> {
        self.check_node(i, j)?;
        Ok(self.values[[i, j]])
    }

    /// Summary statistics over finite primary values.
    pub fn stats(&self) -> Stats {
        Stats::of(&self.values.iter().copied().collect::<Vec<_>>())
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

    /// Human-readable operation history.
    pub fn history(&self) -> &[String] {
        self.history.entries()
    }

    pub(crate) fn set_history(&mut self, history: impl Into<OperationHistory>) {
        self.history = history.into();
    }

    fn check_node(&self, i: usize, j: usize) -> Result<()> {
        if i >= self.ncol || j >= self.nrow {
            return Err(GeoError::OutOfRange(format!(
                "structured surface node ({i}, {j}) outside shape (ncol={}, nrow={})",
                self.ncol, self.nrow
            )));
        }
        Ok(())
    }
}

impl HasHistory for StructuredMeshSurface {
    fn operation_history(&self) -> &OperationHistory {
        &self.history
    }

    fn operation_history_mut(&mut self) -> &mut OperationHistory {
        &mut self.history
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ndarray::array;

    #[test]
    fn keeps_explicit_shifted_node_coordinates() {
        let x = array![[0.0, 0.0], [10.0, 12.0]];
        let y = array![[0.0, 10.0], [0.0, 10.0]];
        let z = array![[100.0, 110.0], [120.0, 130.0]];
        let edge =
            PolygonSet::convex_hull_xy(vec![[0.0, 0.0], [10.0, 0.0], [12.0, 10.0], [0.0, 10.0]])
                .unwrap();

        let s = StructuredMeshSurface::new(x, y, z, None, edge).unwrap();
        assert_eq!(s.kind(), "structured_mesh");
        assert_eq!(s.ncol(), 2);
        assert_eq!(s.nrow(), 2);
        assert_eq!(s.node_xy(1, 1).unwrap(), (12.0, 10.0));
        assert_eq!(s.z(1, 1).unwrap(), 130.0);
        assert_eq!(s.stats().count, 4);
    }
}
