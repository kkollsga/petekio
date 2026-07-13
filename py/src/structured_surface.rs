//! `StructuredMeshSurface` bindings — logical row/column topology with explicit
//! per-node XY coordinates: a shared `StructuredShell` (geometry) + primary
//! values + attribute lanes.

use crate::geodata::GeoData;
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

enum StructuredSurfaceBacking {
    Owned(Arc<RsStructuredMeshSurface>),
    InGeo { geo: Py<GeoData>, name: String },
}

/// A structured mesh surface: regular logical topology, explicit XY nodes.
#[pyclass(name = "StructuredMeshSurface")]
pub struct StructuredMeshSurface {
    backing: StructuredSurfaceBacking,
    name: Option<String>,
}

impl StructuredMeshSurface {
    pub(crate) fn wrap(inner: RsStructuredMeshSurface) -> StructuredMeshSurface {
        StructuredMeshSurface {
            backing: StructuredSurfaceBacking::Owned(Arc::new(inner)),
            name: None,
        }
    }

    /// A cheap view into a project's structured surface (no shell/lane copy).
    pub(crate) fn view(geo: Py<GeoData>, name: String) -> StructuredMeshSurface {
        let display = crate::leaf_name(&name);
        StructuredMeshSurface {
            backing: StructuredSurfaceBacking::InGeo { geo, name },
            name: Some(display),
        }
    }

    /// Attach a dataset display name (the duck-typed viewer seam).
    pub(crate) fn named(mut self, name: Option<String>) -> StructuredMeshSurface {
        self.name = name;
        self
    }

    fn with<R>(
        &self,
        py: Python<'_>,
        f: impl FnOnce(&RsStructuredMeshSurface) -> R,
    ) -> PyResult<R> {
        match &self.backing {
            StructuredSurfaceBacking::Owned(surface) => Ok(f(surface)),
            StructuredSurfaceBacking::InGeo { geo, name } => {
                let geo = geo.borrow(py);
                let surface = geo.inner.structured_surface(name).ok_or_else(|| {
                    PyValueError::new_err(format!("no structured surface '{name}'"))
                })?;
                Ok(f(surface))
            }
        }
    }
}

#[pymethods]
impl StructuredMeshSurface {
    /// Load an EarthVision grid as a null-preserving structured surface.
    #[staticmethod]
    fn load_earthvision_grid(py: Python<'_>, path: &str) -> PyResult<StructuredMeshSurface> {
        py.detach(|| RsStructuredMeshSurface::load_earthvision_grid(path))
            .map(StructuredMeshSurface::wrap)
            .map_err(to_pyerr)
    }

    #[getter]
    fn kind(&self) -> &'static str {
        "structured_mesh"
    }

    /// The dataset name this surface derives from (propagated from the source
    /// point set / surface), or `None` for anonymous meshes. Duck-typed
    /// viewer seam.
    #[getter]
    fn name(&self) -> Option<String> {
        self.name.clone()
    }

    #[getter]
    fn ncol(&self, py: Python<'_>) -> PyResult<usize> {
        self.with(py, RsStructuredMeshSurface::ncol)
    }

    #[getter]
    fn nrow(&self, py: Python<'_>) -> PyResult<usize> {
        self.with(py, RsStructuredMeshSurface::nrow)
    }

    /// Optional approximate regular geometry. This is metadata, not the
    /// canonical coordinate model.
    #[getter]
    fn nominal_geometry(&self, py: Python<'_>) -> PyResult<Option<GridGeometry>> {
        self.with(py, |surface| {
            surface
                .nominal_geometry()
                .map(|geometry| GridGeometry::with_edge(geometry.clone(), surface.edge().clone()))
        })
    }

    /// Edge polygon in modelling coordinates.
    #[getter]
    fn edge(&self, py: Python<'_>) -> PyResult<PolygonSet> {
        self.with(py, |surface| PolygonSet::owned(surface.edge().clone()))
    }

    /// Axis-aligned bounding box over finite XY nodes.
    fn bbox(&self, py: Python<'_>) -> PyResult<BBox> {
        self.with(py, |surface| BBox::new(surface.bbox()))
    }

    /// World `(x, y)` of logical node `(i, j)`.
    fn node_xy(&self, py: Python<'_>, i: usize, j: usize) -> PyResult<(f64, f64)> {
        self.with(py, |surface| surface.node_xy(i, j))?
            .map_err(to_pyerr)
    }

    /// Primary value at logical node `(i, j)`.
    fn z(&self, py: Python<'_>, i: usize, j: usize) -> PyResult<f64> {
        self.with(py, |surface| surface.z(i, j))?.map_err(to_pyerr)
    }

    /// X node coordinates as row-major nested lists: outer list is rows.
    fn x(&self, py: Python<'_>) -> PyResult<Vec<Vec<f64>>> {
        self.with(py, |surface| matrix_rows(surface.x()))
    }

    /// Y node coordinates as row-major nested lists: outer list is rows.
    fn y(&self, py: Python<'_>) -> PyResult<Vec<Vec<f64>>> {
        self.with(py, |surface| matrix_rows(surface.y()))
    }

    /// Z values as row-major nested lists: outer list is rows.
    fn values(&self, py: Python<'_>) -> PyResult<Vec<Vec<f64>>> {
        self.with(py, |surface| matrix_rows(surface.values()))
    }

    /// Explode the mesh back into a `PointSet`, one point per populated node, with
    /// its `column`/`row` topology. Exact — coordinates are copied, not resampled —
    /// so `points.to_structured_surface().to_points()` round-trips losslessly.
    fn to_points(&self, py: Python<'_>) -> PyResult<PointSet> {
        self.with(py, |surface| {
            PointSet::owned(surface.to_points()).named(self.name.clone())
        })
    }

    /// Summary statistics over finite primary values.
    fn stats(&self, py: Python<'_>) -> PyResult<Stats> {
        self.with(py, |surface| Stats::new(surface.stats()))
    }

    /// Human-readable operation history.
    fn history(&self, py: Python<'_>) -> PyResult<Vec<String>> {
        self.with(py, |surface| surface.history().to_vec())
    }

    // ---- the geometry shell + property lanes ----

    /// The geometry shell (level 2, `Arc`-shared: N properties never repeat
    /// the geometry in memory).
    #[getter]
    fn shell(&self, py: Python<'_>) -> PyResult<StructuredShell> {
        self.with(py, |surface| StructuredShell {
            inner: Arc::clone(surface.shell()),
        })
    }

    /// A named attribute lane promoted to a standalone `StructuredMeshSurface`
    /// on the same shared shell (mirrors `Surface.attr`); raises `KeyError`
    /// if absent.
    fn attr(&self, py: Python<'_>, name: &str) -> PyResult<StructuredMeshSurface> {
        self.with(py, |surface| surface.as_attr_surface(name))?
            .map(|s| StructuredMeshSurface::wrap(s).named(self.name.clone()))
            .ok_or_else(|| {
                pyo3::exceptions::PyKeyError::new_err(format!("no attribute layer '{name}'"))
            })
    }

    /// The names of all attribute lanes, in insertion order.
    fn attr_names(&self, py: Python<'_>) -> PyResult<Vec<String>> {
        self.with(py, |surface| {
            surface
                .attr_names()
                .iter()
                .map(|name| name.to_string())
                .collect()
        })
    }

    /// Set (or replace) attribute `name` from row-major nested lists (the
    /// same shape `values()` returns) — returns a **new**
    /// `StructuredMeshSurface` (surfaces are immutable; the shell is shared).
    fn set_attr(
        &self,
        py: Python<'_>,
        name: &str,
        values: Vec<Vec<f64>>,
    ) -> PyResult<StructuredMeshSurface> {
        let mut out = self.with(py, Clone::clone)?;
        let (ncol, nrow) = (out.ncol(), out.nrow());
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
        out.set_attr(name, lane).map_err(to_pyerr)?;
        Ok(StructuredMeshSurface::wrap(out).named(self.name.clone()))
    }

    // ---- conversions ----

    /// Lift to a `TriSurface` (free, lossless: node identity preserved; all
    /// attribute lanes carried 1:1).
    fn to_tri_surface(&self, py: Python<'_>) -> PyResult<TriSurface> {
        let surface = self.with(py, Clone::clone)?;
        py.detach(|| surface.to_tri_surface())
            .map(|t| TriSurface::wrap(t).named(self.name.clone()))
            .map_err(to_pyerr)
    }

    /// Fit a regular `GridGeometry` (lossy downward conversion); raises when
    /// the mesh is curvilinear.
    #[pyo3(signature = (tolerance = 1e-3))]
    fn infer_grid(&self, py: Python<'_>, tolerance: f64) -> PyResult<GridGeometry> {
        self.with(py, |surface| {
            surface.infer_grid(tolerance).map(|geometry| {
                GridGeometry::with_edge(geometry, surface.edge().clone())
                    .named(self.name.as_ref().map(|name| format!("{name} geometry")))
            })
        })?
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
        let surface = self.with(py, Clone::clone)?;
        py.detach(|| surface.resample(&t, gm))
            .map(|s| crate::surface::Surface::wrap(s).named(self.name.clone()))
            .map_err(to_pyerr)
    }

    // ---- iso-lines + value layer ----

    /// Iso-lines of a property lane: `[(level, [[(x, y), ...], ...]), ...]`.
    /// Explicit `levels` win over `interval` (levels aligned to interval
    /// multiples across the value range). NaN-aware: holes break lines.
    /// `simplify=tol` runs Douglas–Peucker on each polyline (world-unit
    /// tolerance; endpoints + ring closure preserved).
    #[pyo3(signature = (interval = None, levels = None, attr = None, simplify = None))]
    fn iso_lines(
        &self,
        py: Python<'_>,
        interval: Option<f64>,
        levels: Option<Vec<f64>>,
        attr: Option<&str>,
        simplify: Option<f64>,
    ) -> PyResult<PyIsoLines> {
        let surface = self.with(py, Clone::clone)?;
        py.detach(|| surface.iso_lines(interval, levels, attr, simplify))
            .map(iso_lines_py)
            .map_err(to_pyerr)
    }

    /// A property lane as the viewer's trimesh dict: `{"kind": "trimesh",
    /// "name", "nodes", "triangles", "values", "range"}`. `stride=k` returns
    /// the coarse-LOD decimation (per-block `(i,j)` striding; `range` from the
    /// full-resolution lane). Display-only.
    #[pyo3(signature = (attr = None, stride = None))]
    fn value_layer(
        &self,
        py: Python<'_>,
        attr: Option<&str>,
        stride: Option<usize>,
    ) -> PyResult<Py<PyDict>> {
        let layer = self
            .with(py, |surface| surface.value_layer(attr, stride))?
            .map_err(to_pyerr)?;
        value_layer_dict(py, layer)
    }

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        self.with(py, |surface| {
            format!(
                "StructuredMeshSurface(ncol={}, nrow={})",
                surface.ncol(),
                surface.nrow()
            )
        })
    }
}
