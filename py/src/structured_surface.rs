//! `StructuredMeshSurface` bindings — logical row/column topology with explicit
//! per-node XY coordinates.

use crate::geometry::{BBox, GridGeometry};
use crate::points::PolygonSet;
use crate::stats::Stats;
use petekio::StructuredMeshSurface as RsStructuredMeshSurface;
use pyo3::prelude::*;
use std::sync::Arc;

/// A structured mesh surface: regular logical topology, explicit XY nodes.
#[pyclass(name = "StructuredMeshSurface")]
pub struct StructuredMeshSurface {
    inner: Arc<RsStructuredMeshSurface>,
}

impl StructuredMeshSurface {
    pub(crate) fn wrap(inner: RsStructuredMeshSurface) -> StructuredMeshSurface {
        StructuredMeshSurface {
            inner: Arc::new(inner),
        }
    }
}

#[pymethods]
impl StructuredMeshSurface {
    #[getter]
    fn kind(&self) -> &'static str {
        self.inner.kind()
    }

    #[getter]
    fn ncol(&self) -> usize {
        self.inner.ncol()
    }

    #[getter]
    fn nrow(&self) -> usize {
        self.inner.nrow()
    }

    /// Optional approximate regular geometry. This is metadata, not the
    /// canonical coordinate model.
    #[getter]
    fn nominal_geometry(&self) -> Option<GridGeometry> {
        self.inner
            .nominal_geometry()
            .map(|g| GridGeometry::with_edge(g.clone(), self.inner.edge().clone()))
    }

    /// Edge polygon in modelling coordinates.
    #[getter]
    fn edge(&self) -> PolygonSet {
        PolygonSet::owned(self.inner.edge().clone())
    }

    /// Axis-aligned bounding box over finite XY nodes.
    fn bbox(&self) -> BBox {
        BBox::new(self.inner.bbox())
    }

    /// World `(x, y)` of logical node `(i, j)`.
    fn node_xy(&self, i: usize, j: usize) -> PyResult<(f64, f64)> {
        self.inner.node_xy(i, j).map_err(crate::to_pyerr)
    }

    /// Primary value at logical node `(i, j)`.
    fn z(&self, i: usize, j: usize) -> PyResult<f64> {
        self.inner.z(i, j).map_err(crate::to_pyerr)
    }

    /// X node coordinates as row-major nested lists: outer list is rows.
    fn x(&self) -> Vec<Vec<f64>> {
        matrix_rows(self.inner.x())
    }

    /// Y node coordinates as row-major nested lists: outer list is rows.
    fn y(&self) -> Vec<Vec<f64>> {
        matrix_rows(self.inner.y())
    }

    /// Z values as row-major nested lists: outer list is rows.
    fn values(&self) -> Vec<Vec<f64>> {
        matrix_rows(self.inner.values())
    }

    /// Summary statistics over finite primary values.
    fn stats(&self) -> Stats {
        Stats::new(self.inner.stats())
    }

    /// Human-readable operation history.
    fn history(&self) -> Vec<String> {
        self.inner.history().to_vec()
    }

    fn __repr__(&self) -> String {
        format!(
            "StructuredMeshSurface(ncol={}, nrow={})",
            self.inner.ncol(),
            self.inner.nrow()
        )
    }
}

fn matrix_rows(a: &ndarray::Array2<f64>) -> Vec<Vec<f64>> {
    let (ncol, nrow) = a.dim();
    (0..nrow)
        .map(|j| (0..ncol).map(|i| a[[i, j]]).collect())
        .collect()
}
