//! `TriSurface` bindings — the triangulated fallback for fault-cut surfaces.

use crate::geometry::BBox;
use crate::points::{PointSet, PolygonSet};
use crate::stats::Stats;
use petekio::TriSurface as RsTriSurface;
use pyo3::prelude::*;
use std::sync::Arc;

/// An unstructured triangulated surface over the original points.
#[pyclass(name = "TriSurface")]
pub struct TriSurface {
    inner: Arc<RsTriSurface>,
}

impl TriSurface {
    pub(crate) fn wrap(inner: RsTriSurface) -> TriSurface {
        TriSurface {
            inner: Arc::new(inner),
        }
    }
}

#[pymethods]
impl TriSurface {
    #[getter]
    fn kind(&self) -> &'static str {
        self.inner.kind()
    }

    #[getter]
    fn n_points(&self) -> usize {
        self.inner.points().len()
    }

    #[getter]
    fn n_triangles(&self) -> usize {
        self.inner.triangles().len()
    }

    /// Vertices as `(x, y, z)` tuples — the input points, unmoved.
    fn points(&self) -> Vec<(f64, f64, f64)> {
        self.inner
            .points()
            .iter()
            .map(|p| (p[0], p[1], p[2]))
            .collect()
    }

    /// Triangles as `(i, j, k)` index triples into `points()`.
    fn triangles(&self) -> Vec<(u32, u32, u32)> {
        self.inner
            .triangles()
            .iter()
            .map(|t| (t[0], t[1], t[2]))
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
        PointSet::owned(self.inner.to_points())
    }

    /// Human-readable operation history.
    fn history(&self) -> Vec<String> {
        self.inner.history().to_vec()
    }

    fn __repr__(&self) -> String {
        format!(
            "TriSurface(points={}, triangles={}, rings={})",
            self.inner.points().len(),
            self.inner.triangles().len(),
            self.inner.edge().rings().len(),
        )
    }
}
