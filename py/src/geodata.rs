//! `GeoData` — the load-once project substrate: named surfaces, wells, points,
//! and polygons under one declared length unit. Mirrors `petekio::GeoData`.
//!
//! Surfaces are handed back as owned copies (deep-cloned); points/polygons/wells
//! are returned as lightweight **views** that re-resolve into this project's
//! collections by name. The wells view (`geo.wells()`) is the broadcast
//! substrate — see `well.rs`.

use crate::points::{PointSet, PolygonSet};
use crate::stats::Stats;
use crate::surface::{clone_surface, Surface};
use crate::well::Well;
use crate::{parse_unit, to_pyerr};
use petekio::GeoData as RsGeoData;
use pyo3::exceptions::{PyAttributeError, PyValueError};
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

    /// Load a well from `files` (a per-well directory or single file) under
    /// `id`, returning a view. `head` (wellhead `(x, y)`) and `kb` (datum) are
    /// **optional** — when a `.wellpath` is present its header is authoritative
    /// and overrides them, so for a positioned well you can omit both:
    /// `geo.load_well("15/9-A1", files="wells/")`. Without a `.wellpath` they
    /// default to `(0, 0)` / `0`.
    #[pyo3(signature = (id, head=None, kb=None, files=None))]
    fn load_well(
        slf: Bound<'_, Self>,
        id: &str,
        head: Option<(f64, f64)>,
        kb: Option<f64>,
        files: Option<&str>,
    ) -> PyResult<Well> {
        let files = files.ok_or_else(|| {
            PyValueError::new_err("load_well: `files` (a well directory or file) is required")
        })?;
        slf.borrow_mut()
            .inner
            .load_well(id, head.unwrap_or((0.0, 0.0)), kb.unwrap_or(0.0), files)
            .map_err(to_pyerr)?;
        Ok(Well::view(slf.clone().unbind(), id.to_string()))
    }

    /// Load a multi-well Petrel well-tops file; route each `Horizon` pick to the
    /// matching loaded well + bore. Returns the number of tops assigned. Also
    /// derives the project's global lithostratigraphic column (see
    /// `strat_order`) across every well in the file and pushes it into each
    /// loaded well, so `zones()`/`zone_stats()` present in that order.
    fn load_well_tops(&mut self, path: &str) -> PyResult<usize> {
        self.inner.load_well_tops(path).map_err(to_pyerr)
    }

    /// The global lithostratigraphic column (top names, shallow→deep) derived by
    /// the last `load_well_tops` across every well in that file. Empty list
    /// before any tops are loaded.
    #[getter]
    fn strat_order(&self) -> Vec<String> {
        self.inner.strat_order().to_vec()
    }

    /// The well stored under `id` (view), or `None`.
    fn well(slf: Bound<'_, Self>, id: &str) -> Option<Well> {
        slf.borrow()
            .inner
            .well(id)
            .is_some()
            .then(|| Well::view(slf.clone().unbind(), id.to_string()))
    }

    /// All surfaces in insertion order, as owned copies.
    fn surfaces(&self) -> Vec<Surface> {
        self.inner
            .surfaces()
            .map(|s| Surface::wrap(clone_surface(s)))
            .collect()
    }

    /// A broadcastable, filterable view over all wells (insertion order).
    #[getter]
    fn wells(slf: Bound<'_, Self>) -> WellsView {
        let ids: Vec<String> = slf
            .borrow()
            .inner
            .wells()
            .iter()
            .map(|w| w.id.clone())
            .collect();
        WellsView::new(slf.clone().unbind(), ids, None)
    }
}

/// A lightweight, broadcastable, filterable view over a project's wells.
///
/// Holds the owning `GeoData` plus the well ids in view. `filter`/`tops` narrow
/// the set; `__getattr__` broadcasts: with no pending top it resolves a top name
/// (returning a narrowed view), and after `tops(name)` it resolves a log
/// mnemonic to a per-well `list[Stats]`.
#[pyclass(name = "WellsView")]
pub struct WellsView {
    geo: Py<GeoData>,
    ids: Vec<String>,
    /// The top set by a prior `tops(...)`/attribute step, if any.
    current_top: Option<String>,
}

impl WellsView {
    fn new(geo: Py<GeoData>, ids: Vec<String>, current_top: Option<String>) -> WellsView {
        WellsView {
            geo,
            ids,
            current_top,
        }
    }
}

#[pymethods]
impl WellsView {
    fn __len__(&self) -> usize {
        self.ids.len()
    }

    fn len(&self) -> usize {
        self.ids.len()
    }

    fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    /// The wells in this view as `Well` objects (insertion order).
    fn iter(&self, py: Python<'_>) -> Vec<Well> {
        self.ids
            .iter()
            .map(|id| Well::view(self.geo.clone_ref(py), id.clone()))
            .collect()
    }

    /// A new view keeping only wells for which `pred(well)` is truthy.
    fn filter(&self, py: Python<'_>, pred: &Bound<'_, PyAny>) -> PyResult<WellsView> {
        let mut kept = Vec::new();
        for id in &self.ids {
            let w = Well::view(self.geo.clone_ref(py), id.clone());
            if pred.call1((w,))?.is_truthy()? {
                kept.push(id.clone());
            }
        }
        Ok(WellsView::new(
            self.geo.clone_ref(py),
            kept,
            self.current_top.clone(),
        ))
    }

    /// A new view narrowed to wells that have the named top; remembers the top
    /// so a following attribute access resolves a log to a per-well `Stats`.
    fn tops(&self, py: Python<'_>, name: &str) -> WellsView {
        let g = self.geo.borrow(py);
        let kept: Vec<String> = self
            .ids
            .iter()
            .filter(|id| {
                g.inner
                    .well(id)
                    .map(|w| w.top(name).is_some())
                    .unwrap_or(false)
            })
            .cloned()
            .collect();
        drop(g);
        WellsView::new(self.geo.clone_ref(py), kept, Some(name.to_string()))
    }

    /// Broadcast attribute access. With no pending top, `name` is a top marker →
    /// a narrowed view (like `tops`). After `tops(name)`, `name` is a log
    /// mnemonic → a per-well `list[Stats]` over that top's interval.
    fn __getattr__(&self, py: Python<'_>, name: &str) -> PyResult<Py<PyAny>> {
        if name.starts_with('_') {
            return Err(PyAttributeError::new_err(name.to_string()));
        }
        match &self.current_top {
            None => {
                let view = self.tops(py, name);
                if view.ids.is_empty() {
                    return Err(PyAttributeError::new_err(format!(
                        "no well in this view carries top '{name}'"
                    )));
                }
                Ok(view.into_pyobject(py)?.into_any().unbind())
            }
            Some(top) => {
                let g = self.geo.borrow(py);
                let mut out: Vec<Stats> = Vec::new();
                for id in &self.ids {
                    if let Some(stats) = g
                        .inner
                        .well(id)
                        .and_then(|w| w.top(top))
                        .and_then(|iv| iv.log(name).map(|lv| lv.stats()))
                    {
                        out.push(Stats::new(stats));
                    }
                }
                Ok(out.into_pyobject(py)?.into_any().unbind())
            }
        }
    }

    fn __repr__(&self) -> String {
        format!("WellsView(len={})", self.ids.len())
    }
}
