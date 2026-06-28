//! `GeoData` — the load-once project substrate: named surfaces, wells, points,
//! and polygons under one declared length unit. Mirrors `petekio::GeoData`.
//!
//! Surfaces are handed back as owned copies (deep-cloned); points/polygons/wells
//! are returned as lightweight **views** that re-resolve into this project's
//! collections by name. The wells view (`geo.wells()`) is the broadcast
//! substrate — see `well.rs`.

use crate::points::{PointSet, PolygonSet};
use crate::surface::{clone_surface, Surface};
use crate::{parse_unit, to_pyerr};
use petekio::GeoData as RsGeoData;
use pyo3::prelude::*;

/// A load-once subsurface project under one declared length unit.
#[pyclass(name = "GeoData")]
pub struct GeoData {
    pub(crate) inner: RsGeoData,
}

#[pymethods]
impl GeoData {
    /// An empty project. `unit` is `"ft"`/`"feet"` or `"m"`/`"metres"`.
    #[new]
    #[pyo3(signature = (unit = "m"))]
    fn py_new(unit: &str) -> PyResult<GeoData> {
        Ok(GeoData {
            inner: RsGeoData::new(parse_unit(unit)?),
        })
    }

    /// The project's length unit, as a string (`"ft"` or `"m"`).
    #[getter]
    fn unit(&self) -> &'static str {
        match self.inner.unit {
            petekio::Unit::Feet => "ft",
            petekio::Unit::Metres => "m",
        }
    }

    /// Load a surface from `path` (IRAP classic) and store it under `name`,
    /// returning an owned copy.
    fn load_surface(&mut self, name: &str, path: &str) -> PyResult<Surface> {
        self.inner.load_surface(name, path).map_err(to_pyerr)?;
        let s = self
            .inner
            .surface(name)
            .expect("just-loaded surface is present");
        Ok(Surface::wrap(clone_surface(s)))
    }

    /// Load a point set from `path` (extension-dispatched) under `name`,
    /// returning a view.
    fn load_points(slf: Bound<'_, Self>, name: &str, path: &str) -> PyResult<PointSet> {
        slf.borrow_mut()
            .inner
            .load_points(name, path)
            .map_err(to_pyerr)?;
        Ok(PointSet::view(slf.clone().unbind(), name.to_string()))
    }

    /// Load a polygon set from `path` (extension-dispatched) under `name`,
    /// returning a view.
    fn load_polygons(slf: Bound<'_, Self>, name: &str, path: &str) -> PyResult<PolygonSet> {
        slf.borrow_mut()
            .inner
            .load_polygons(name, path)
            .map_err(to_pyerr)?;
        Ok(PolygonSet::view(slf.clone().unbind(), name.to_string()))
    }

    /// The surface stored under `name` (owned copy), or `None`.
    fn surface(&self, name: &str) -> Option<Surface> {
        self.inner
            .surface(name)
            .map(|s| Surface::wrap(clone_surface(s)))
    }

    /// The point set stored under `name` (view), or `None`.
    fn points(slf: Bound<'_, Self>, name: &str) -> Option<PointSet> {
        slf.borrow()
            .inner
            .points(name)
            .is_some()
            .then(|| PointSet::view(slf.clone().unbind(), name.to_string()))
    }

    /// The polygon set stored under `name` (view), or `None`.
    fn polygons(slf: Bound<'_, Self>, name: &str) -> Option<PolygonSet> {
        slf.borrow()
            .inner
            .polygons(name)
            .is_some()
            .then(|| PolygonSet::view(slf.clone().unbind(), name.to_string()))
    }

    /// All surfaces in insertion order, as owned copies.
    fn surfaces(&self) -> Vec<Surface> {
        self.inner
            .surfaces()
            .map(|s| Surface::wrap(clone_surface(s)))
            .collect()
    }
}
