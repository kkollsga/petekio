//! `TriSurface` bindings — the triangulated fallback for fault-cut surfaces:
//! a shared `MeshShell` (geometry) + primary z values + attribute lanes.

use crate::attribute::{metadata_from_dict, metadata_to_dict};
use crate::geometry::{BBox, GridGeometry};
use crate::points::{PointSet, PolygonSet};
use crate::shell::{iso_lines_py, value_layer_dict, MeshShell, PyIsoLines};
use crate::stats::Stats;
use crate::{parse_grid_method, to_pyerr};
use petekio::TriSurface as RsTriSurface;
use pyo3::prelude::*;
use pyo3::types::PyDict;
use std::sync::Arc;

/// An unstructured triangulated surface over the original points.
#[pyclass(name = "TriSurface")]
pub struct TriSurface {
    inner: Arc<RsTriSurface>,
    name: Option<String>,
}

impl TriSurface {
    pub(crate) fn wrap(inner: RsTriSurface) -> TriSurface {
        TriSurface {
            inner: Arc::new(inner),
            name: None,
        }
    }

    /// Attach a dataset display name (the duck-typed viewer seam).
    pub(crate) fn named(mut self, name: Option<String>) -> TriSurface {
        self.name = name;
        self
    }

    pub(crate) fn with<R>(&self, f: impl FnOnce(&RsTriSurface) -> R) -> R {
        f(&self.inner)
    }

    pub(crate) fn dataset_name(&self) -> Option<String> {
        self.name.clone()
    }
}

#[pymethods]
impl TriSurface {
    #[getter]
    fn kind(&self) -> &'static str {
        self.inner.kind()
    }

    /// The dataset name this surface derives from (propagated from the source
    /// point set / surface), or `None` for anonymous meshes. Duck-typed
    /// viewer seam.
    #[getter]
    fn name(&self) -> Option<String> {
        self.name.clone()
    }

    #[getter]
    fn n_points(&self) -> usize {
        self.inner.shell().n_nodes()
    }

    #[getter]
    fn n_triangles(&self) -> usize {
        self.inner.triangles().len()
    }

    /// Connected components; more than one means the mesh honours a fault.
    #[getter]
    fn components(&self) -> usize {
        self.inner.components()
    }

    /// Vertices as `(x, y, z)` tuples — the input points, unmoved.
    fn points(&self) -> Vec<(f64, f64, f64)> {
        self.inner
            .points()
            .iter()
            .map(|p| (p[0], p[1], p[2]))
            .collect()
    }

    /// Vertices through the generic point/viewer protocol.
    fn xyz(&self) -> Vec<(f64, f64, f64)> {
        self.points()
    }

    /// Triangles as `(i, j, k)` index triples into `points()`.
    fn triangles(&self) -> Vec<(u32, u32, u32)> {
        self.inner
            .triangles()
            .iter()
            .map(|t| (t[0], t[1], t[2]))
            .collect()
    }

    /// Unique triangle edges minus interior cell diagonals, as `(i, j)` index
    /// pairs into `points()` — the quad-dominant wireframe (a full lattice
    /// cell draws as a square). `stride=k` (k ≥ 2) returns the coarse-LOD
    /// lattice wireframe (every k-th grid line per block, outline + seams +
    /// fringe kept); `None`/`1` is the full wireframe. Display-only.
    #[pyo3(signature = (stride = None))]
    fn wireframe_edges(&self, stride: Option<usize>) -> Vec<(u32, u32)> {
        self.inner
            .wireframe_edges(stride)
            .into_iter()
            .map(|e| (e[0], e[1]))
            .collect()
    }

    /// Outer boundary ring(s) of the retained triangles.
    #[getter]
    fn edge(&self) -> PolygonSet {
        PolygonSet::owned(self.inner.edge().clone())
    }

    /// Statistics over the vertices' z.
    fn stats(&self) -> Stats {
        Stats::new(self.inner.stats())
    }

    /// Axis-aligned bounding box over the vertices' XY.
    fn bbox(&self) -> BBox {
        BBox::new(self.inner.bbox())
    }

    /// The vertices as a `PointSet` — exact, nothing resampled.
    fn to_points(&self) -> PointSet {
        PointSet::owned(self.inner.to_points()).named(self.name.clone())
    }

    /// Human-readable operation history.
    fn history(&self) -> Vec<String> {
        self.inner.history().to_vec()
    }

    // ---- the geometry shell + property lanes ----

    /// The geometry shell (level 3, `Arc`-shared: N properties never repeat
    /// the geometry in memory).
    #[getter]
    fn shell(&self) -> MeshShell {
        MeshShell::wrap(Arc::clone(self.inner.shell()))
            .named(self.name.as_ref().map(|name| format!("{name} geometry")))
    }

    /// The primary per-node values (z). `NaN` = undefined.
    fn values(&self) -> Vec<f64> {
        self.inner.values().to_vec()
    }

    /// A named attribute lane promoted to a standalone `TriSurface` on the
    /// same shared shell (mirrors `Surface.attr`); raises `KeyError` if absent.
    fn attr(&self, name: &str) -> PyResult<TriSurface> {
        self.inner
            .as_attr_surface(name)
            .map(|t| TriSurface::wrap(t).named(self.name.clone()))
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

    /// Canonical durable metadata for attribute `name`.
    fn attr_metadata(&self, py: Python<'_>, name: &str) -> PyResult<Py<PyDict>> {
        let metadata = self.inner.attr_metadata(name).ok_or_else(|| {
            pyo3::exceptions::PyKeyError::new_err(format!("no attribute layer '{name}'"))
        })?;
        Ok(metadata_to_dict(py, metadata)?.unbind())
    }

    /// Metadata of the promoted primary lane, if this surface is an attribute.
    #[getter]
    fn primary_metadata(&self, py: Python<'_>) -> PyResult<Option<Py<PyDict>>> {
        self.inner
            .primary_metadata()
            .map(|metadata| metadata_to_dict(py, metadata).map(Bound::unbind))
            .transpose()
    }

    /// Set (or replace) attribute `name` (one value per node) — returns a
    /// **new** `TriSurface` (surfaces are immutable; the shell is shared).
    #[pyo3(signature = (name, values, metadata = None))]
    fn set_attr(
        &self,
        name: &str,
        values: Vec<f64>,
        metadata: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<TriSurface> {
        let mut out = (*self.inner).clone();
        match metadata {
            Some(metadata) => out
                .set_attr_with_metadata(name, values, metadata_from_dict(name, metadata)?)
                .map_err(to_pyerr)?,
            None => out.set_attr(name, values).map_err(to_pyerr)?,
        }
        Ok(TriSurface::wrap(out).named(self.name.clone()))
    }

    // ---- conversions ----

    /// Fit a regular `GridGeometry` (lossy downward conversion); raises when
    /// the mesh is not regular.
    #[pyo3(signature = (tolerance = 1e-3))]
    fn infer_grid(&self, tolerance: f64) -> PyResult<GridGeometry> {
        self.inner
            .infer_grid(tolerance)
            .map(|g| {
                GridGeometry::with_edge(g, self.inner.edge().clone())
                    .named(self.name.as_ref().map(|n| format!("{n} geometry")))
            })
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
        py.detach(|| self.inner.iso_lines(interval, levels, attr, simplify))
            .map(iso_lines_py)
            .map_err(to_pyerr)
    }

    /// A property lane as the viewer's trimesh dict: `{"kind": "trimesh",
    /// "name", "nodes", "triangles", "values", "range"}`. `stride=k` returns
    /// the coarse-LOD decimation (per-block `(i,j)`-label striding; `range`
    /// from the full-resolution lane). Display-only.
    #[pyo3(signature = (attr = None, stride = None))]
    fn value_layer(
        &self,
        py: Python<'_>,
        attr: Option<&str>,
        stride: Option<usize>,
    ) -> PyResult<Py<PyDict>> {
        let layer = self.inner.value_layer(attr, stride).map_err(to_pyerr)?;
        value_layer_dict(py, layer)
    }

    fn __repr__(&self) -> String {
        format!(
            "TriSurface(points={}, triangles={}, components={}, rings={})",
            self.inner.shell().n_nodes(),
            self.inner.triangles().len(),
            self.inner.components(),
            self.inner.edge().rings().len(),
        )
    }
}
