//! Python value/result objects for well/surface intersections.

use crate::geodata::GeoData;
use crate::structured_surface::StructuredMeshSurface;
use crate::surface::Surface;
use crate::to_pyerr;
use crate::tri_surface::TriSurface;
use petekio::{
    IntersectableSurface, IntersectionDiagnostic, SurfaceIntersection as RsIntersection,
    WellIntersectionSet as RsIntersectionSet,
};
use pyo3::exceptions::{PyIndexError, PyKeyError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyAny, PyDict};

pub(crate) fn with_surface<R>(
    py: Python<'_>,
    obj: &Bound<'_, PyAny>,
    f: impl FnOnce(&dyn IntersectableSurface) -> petekio::Result<R>,
) -> PyResult<(R, Option<String>)> {
    if let Ok(surface) = obj.extract::<PyRef<'_, Surface>>() {
        let name = surface.dataset_name();
        return surface
            .with(py, |inner| f(inner))?
            .map(|value| (value, name))
            .map_err(to_pyerr);
    }
    if let Ok(surface) = obj.extract::<PyRef<'_, StructuredMeshSurface>>() {
        let name = surface.dataset_name();
        return surface
            .with(py, |inner| f(inner))?
            .map(|value| (value, name))
            .map_err(to_pyerr);
    }
    if let Ok(surface) = obj.extract::<PyRef<'_, TriSurface>>() {
        let name = surface.dataset_name();
        return surface
            .with(|inner| f(inner))
            .map(|value| (value, name))
            .map_err(to_pyerr);
    }
    Err(pyo3::exceptions::PyTypeError::new_err(
        "surface must be Surface, StructuredMeshSurface, or TriSurface",
    ))
}

/// One immutable crossing with measured depth, XYZ, and source identity.
#[pyclass(name = "SurfaceIntersection", frozen, skip_from_py_object)]
#[derive(Clone)]
pub struct SurfaceIntersection {
    pub(crate) inner: RsIntersection,
    pub(crate) project_token: Option<usize>,
}

impl SurfaceIntersection {
    pub(crate) fn attach_surface(mut inner: RsIntersection, surface: Option<&str>) -> Self {
        inner.surface = surface.map(str::to_string);
        Self {
            inner,
            project_token: None,
        }
    }

    pub(crate) fn attach_project(
        mut inner: RsIntersection,
        surface: Option<&str>,
        project_token: usize,
    ) -> Self {
        inner.surface = surface.map(str::to_string);
        Self {
            inner,
            project_token: Some(project_token),
        }
    }
}

#[pymethods]
impl SurfaceIntersection {
    #[getter]
    fn md(&self) -> f64 {
        self.inner.md
    }

    #[getter]
    fn xyz(&self) -> (f64, f64, f64) {
        (self.inner.xyz.x, self.inner.xyz.y, self.inner.xyz.z)
    }

    #[getter]
    fn well(&self) -> Option<&str> {
        self.inner.well.as_deref()
    }

    #[getter]
    fn bore(&self) -> Option<&str> {
        self.inner.bore.as_deref()
    }

    #[getter]
    fn surface(&self) -> Option<&str> {
        self.inner.surface.as_deref()
    }

    fn to_dict(&self, py: Python<'_>) -> PyResult<Py<PyDict>> {
        let out = PyDict::new(py);
        out.set_item("md", self.md())?;
        out.set_item("xyz", self.xyz())?;
        out.set_item("well", self.well())?;
        out.set_item("bore", self.bore())?;
        out.set_item("surface", self.surface())?;
        Ok(out.unbind())
    }

    fn keys(&self) -> [&'static str; 5] {
        ["md", "xyz", "well", "bore", "surface"]
    }

    fn __getitem__(&self, py: Python<'_>, key: &str) -> PyResult<Py<PyAny>> {
        match key {
            "md" => Ok(self.md().into_pyobject(py)?.into_any().unbind()),
            "xyz" => Ok(self.xyz().into_pyobject(py)?.into_any().unbind()),
            "well" => Ok(self.well().into_pyobject(py)?.into_any().unbind()),
            "bore" => Ok(self.bore().into_pyobject(py)?.into_any().unbind()),
            "surface" => Ok(self.surface().into_pyobject(py)?.into_any().unbind()),
            _ => Err(PyKeyError::new_err(key.to_string())),
        }
    }

    fn __repr__(&self) -> String {
        format!(
            "SurfaceIntersection(md={:.6}, xyz=({:.6}, {:.6}, {:.6}), well={:?}, bore={:?}, surface={:?})",
            self.inner.md,
            self.inner.xyz.x,
            self.inner.xyz.y,
            self.inner.xyz.z,
            self.inner.well,
            self.inner.bore,
            self.inner.surface,
        )
    }
}

/// Aggregate returned by `WellsView.intersection(s)`.
#[pyclass(name = "WellIntersectionSet")]
pub struct WellIntersectionSet {
    pub(crate) inner: RsIntersectionSet,
    source: Py<GeoData>,
    full_scope: bool,
}

impl WellIntersectionSet {
    pub(crate) fn new(inner: RsIntersectionSet, source: Py<GeoData>, full_scope: bool) -> Self {
        Self {
            inner,
            source,
            full_scope,
        }
    }
}

#[pymethods]
impl WellIntersectionSet {
    #[getter]
    fn hits(&self) -> Vec<SurfaceIntersection> {
        let token = self.source.as_ptr() as usize;
        self.inner
            .hits
            .iter()
            .cloned()
            .map(|inner| SurfaceIntersection {
                inner,
                project_token: Some(token),
            })
            .collect()
    }

    #[getter]
    fn skipped(&self, py: Python<'_>) -> PyResult<Vec<Py<PyDict>>> {
        diagnostics(py, &self.inner.skipped)
    }

    #[getter]
    fn failed(&self, py: Python<'_>) -> PyResult<Vec<Py<PyDict>>> {
        diagnostics(py, &self.inner.failed)
    }

    fn summary(&self, py: Python<'_>) -> PyResult<Py<PyDict>> {
        let (hits, skipped, failed) = self.inner.summary();
        let out = PyDict::new(py);
        out.set_item("hits", hits)?;
        out.set_item("skipped", skipped)?;
        out.set_item("failed", failed)?;
        Ok(out.unbind())
    }

    fn __len__(&self) -> usize {
        self.inner.hits.len()
    }

    fn __getitem__(&self, index: isize) -> PyResult<SurfaceIntersection> {
        let len = self.inner.hits.len() as isize;
        let index = if index < 0 { len + index } else { index };
        self.inner
            .hits
            .get(index as usize)
            .cloned()
            .map(|inner| SurfaceIntersection {
                inner,
                project_token: Some(self.source.as_ptr() as usize),
            })
            .ok_or_else(|| PyIndexError::new_err(index))
    }

    /// Internal atomic persistence seam used by `project.well_tops[name] = rhs`.
    fn _apply_top(&self, py: Python<'_>, expected: Py<GeoData>, name: &str) -> PyResult<()> {
        if self.source.as_ptr() != expected.as_ptr() {
            return Err(PyValueError::new_err(
                "well-top assignment result belongs to a different project",
            ));
        }
        if !self.full_scope {
            return Err(PyValueError::new_err(
                "well-top assignment requires a result from the complete project.wells view",
            ));
        }
        if let Some(first) = self.inner.failed.first() {
            return Err(PyValueError::new_err(format!(
                "well-top assignment blocked by {} failed bore(s); first {}/{}: {}",
                self.inner.failed.len(),
                first.well,
                first.bore,
                first.message
            )));
        }
        expected
            .borrow_mut(py)
            .inner
            .replace_well_top_set(name, &self.inner.hits)
            .map_err(to_pyerr)
    }

    fn __repr__(&self) -> String {
        let (hits, skipped, failed) = self.inner.summary();
        format!("WellIntersectionSet(hits={hits}, skipped={skipped}, failed={failed})")
    }
}

fn diagnostics(py: Python<'_>, rows: &[IntersectionDiagnostic]) -> PyResult<Vec<Py<PyDict>>> {
    rows.iter()
        .map(|row| {
            let out = PyDict::new(py);
            out.set_item("well", &row.well)?;
            out.set_item("bore", &row.bore)?;
            out.set_item("reason", &row.reason)?;
            out.set_item("message", &row.message)?;
            Ok(out.unbind())
        })
        .collect()
}
