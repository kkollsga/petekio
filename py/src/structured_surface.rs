//! `StructuredMeshSurface` bindings — logical row/column topology with explicit
//! per-node XY coordinates: a shared `StructuredShell` (geometry) + primary
//! values + attribute lanes.

use crate::geometry::{BBox, GridGeometry};
use crate::points::{PointSet, PolygonSet};
use crate::shell::{iso_lines_py, matrix_rows, value_layer_dict, PyIsoLines, StructuredShell};
use crate::stats::Stats;
use crate::tri_surface::TriSurface;
use crate::{parse_grid_method, to_pyerr};
use petekio::StructuredMeshSurface as RsStructuredMeshSurface;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyDict;
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
        self.inner.node_xy(i, j).map_err(to_pyerr)
    }

    /// Primary value at logical node `(i, j)`.
    fn z(&self, i: usize, j: usize) -> PyResult<f64> {
        self.inner.z(i, j).map_err(to_pyerr)
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

    /// Explode the mesh back into a `PointSet`, one point per populated node, with
    /// its `column`/`row` topology. Exact — coordinates are copied, not resampled —
    /// so `points.to_structured_surface().to_points()` round-trips losslessly.
    fn to_points(&self) -> PointSet {
        PointSet::owned(self.inner.to_points())
    }

    /// Summary statistics over finite primary values.
    fn stats(&self) -> Stats {
        Stats::new(self.inner.stats())
    }

    /// Human-readable operation history.
    fn history(&self) -> Vec<String> {
        self.inner.history().to_vec()
    }

    // ---- the geometry shell + property lanes ----

    /// The geometry shell (level 2, `Arc`-shared: N properties never repeat
    /// the geometry in memory).
    #[getter]
    fn shell(&self) -> StructuredShell {
        StructuredShell {
            inner: Arc::clone(self.inner.shell()),
        }
    }

    /// A named attribute lane promoted to a standalone `StructuredMeshSurface`
    /// on the same shared shell (mirrors `Surface.attr`); raises `KeyError`
    /// if absent.
    fn attr(&self, name: &str) -> PyResult<StructuredMeshSurface> {
        self.inner
            .as_attr_surface(name)
            .map(StructuredMeshSurface::wrap)
            .ok_or_else(|| {
                pyo3::exceptions::PyKeyError::new_err(format!("no attribute layer '{name}'"))
            })
    }

    /// The names of all attribute lanes, in insertion order.
    fn attr_names(&self) -> Vec<String> {
        self.inner
            .attr_names()
            .iter()
            .map(|n| n.to_string())
            .collect()
    }

    /// Set (or replace) attribute `name` from row-major nested lists (the
    /// same shape `values()` returns) — returns a **new**
    /// `StructuredMeshSurface` (surfaces are immutable; the shell is shared).
    fn set_attr(&self, name: &str, values: Vec<Vec<f64>>) -> PyResult<StructuredMeshSurface> {
        let (ncol, nrow) = (self.inner.ncol(), self.inner.nrow());
        if values.len() != nrow || values.iter().any(|row| row.len() != ncol) {
            return Err(PyValueError::new_err(format!(
                "set_attr expects {nrow} rows of {ncol} values (the shape values() returns)"
            )));
        }
        let mut lane = ndarray::Array2::from_elem((ncol, nrow), f64::NAN);
        for (j, row) in values.iter().enumerate() {
            for (i, v) in row.iter().enumerate() {
                lane[[i, j]] = *v;
            }
        }
        let mut out = (*self.inner).clone();
        out.set_attr(name, lane).map_err(to_pyerr)?;
        Ok(StructuredMeshSurface::wrap(out))
    }

    // ---- conversions ----

    /// Lift to a `TriSurface` (free, lossless: node identity preserved; all
    /// attribute lanes carried 1:1).
    fn to_tri_surface(&self, py: Python<'_>) -> PyResult<TriSurface> {
        py.detach(|| self.inner.to_tri_surface())
            .map(TriSurface::wrap)
            .map_err(to_pyerr)
    }

    /// Fit a regular `GridGeometry` (lossy downward conversion); raises when
    /// the mesh is curvilinear.
    #[pyo3(signature = (tolerance = 1e-3))]
    fn infer_grid(&self, tolerance: f64) -> PyResult<GridGeometry> {
        self.inner
            .infer_grid(tolerance)
            .map(|g| GridGeometry::with_edge(g, self.inner.edge().clone()))
            .map_err(to_pyerr)
    }

    /// Resample the primary values **and every attribute lane** onto a target
    /// regular geometry through the shared gridding kernels. `method`:
    /// `"nearest"`, `"idw"`, or `"min_curvature"`.
    #[pyo3(signature = (target, method = "min_curvature"))]
    fn resample(
        &self,
        py: Python<'_>,
        target: &GridGeometry,
        method: &str,
    ) -> PyResult<crate::surface::Surface> {
        let gm = parse_grid_method(method)?;
        let t = target.inner.clone();
        py.detach(|| self.inner.resample(&t, gm))
            .map(crate::surface::Surface::wrap)
            .map_err(to_pyerr)
    }

    // ---- iso-lines + value layer ----

    /// Iso-lines of a property lane: `[(level, [[(x, y), ...], ...]), ...]`.
    /// Explicit `levels` win over `interval` (levels aligned to interval
    /// multiples across the value range). NaN-aware: holes break lines.
    #[pyo3(signature = (interval = None, levels = None, attr = None))]
    fn iso_lines(
        &self,
        py: Python<'_>,
        interval: Option<f64>,
        levels: Option<Vec<f64>>,
        attr: Option<&str>,
    ) -> PyResult<PyIsoLines> {
        py.detach(|| self.inner.iso_lines(interval, levels, attr))
            .map(iso_lines_py)
            .map_err(to_pyerr)
    }

    /// A property lane as the viewer's trimesh dict: `{"kind": "trimesh",
    /// "name", "nodes", "triangles", "values", "range"}`.
    #[pyo3(signature = (attr = None))]
    fn value_layer(&self, py: Python<'_>, attr: Option<&str>) -> PyResult<Py<PyDict>> {
        let layer = self.inner.value_layer(attr).map_err(to_pyerr)?;
        value_layer_dict(py, layer)
    }

    fn __repr__(&self) -> String {
        format!(
            "StructuredMeshSurface(ncol={}, nrow={})",
            self.inner.ncol(),
            self.inner.nrow()
        )
    }
}
