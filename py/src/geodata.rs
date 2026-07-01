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
use pyo3::types::{PyBytes, PyDict};

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

    /// Add a soft lithostratigraphic ordering hint, honoured by the *next*
    /// `load_well_tops` only where the data leaves the pair unordered (real MD
    /// positions always win). Two forms:
    ///
    /// - shorthand: `geo.strat_hint("Basal < Cerisa West")` — `A < B` is "A
    ///   above B", `A > B` is "A below B"; sides may be partial names.
    /// - explicit: `geo.strat_hint(above="Basal Shale top", below="Cerisa West top")`.
    #[pyo3(signature = (spec=None, *, above=None, below=None))]
    fn strat_hint(
        &mut self,
        spec: Option<&str>,
        above: Option<&str>,
        below: Option<&str>,
    ) -> PyResult<()> {
        match (spec, above, below) {
            (Some(s), None, None) => self.inner.strat_hint(s).map_err(to_pyerr),
            (None, Some(a), Some(b)) => {
                self.inner.add_strat_hint(a, b);
                Ok(())
            }
            (None, None, None) => Err(PyValueError::new_err(
                "strat_hint: pass a shorthand string (e.g. \"A < B\") or above=/below=",
            )),
            _ => Err(PyValueError::new_err(
                "strat_hint: use EITHER a shorthand string OR above=/below=, not both",
            )),
        }
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

    // ---- persistence (.pproj) ---------------------------------------------

    /// Save the whole project to a single `.pproj` file (atomic).
    fn save(&self, path: &str) -> PyResult<()> {
        self.inner.save(path).map_err(to_pyerr)
    }

    /// Open a `.pproj` project.
    #[staticmethod]
    fn open(path: &str) -> PyResult<GeoData> {
        Ok(GeoData {
            inner: RsGeoData::open(path).map_err(to_pyerr)?,
        })
    }

    /// Read a project's manifest (owner/tags/unit/timestamps + element index)
    /// without decoding any element — a dict.
    #[staticmethod]
    fn inspect<'py>(py: Python<'py>, path: &str) -> PyResult<Bound<'py, PyDict>> {
        let info = RsGeoData::inspect(path).map_err(to_pyerr)?;
        let d = PyDict::new(py);
        d.set_item("owner", info.owner)?;
        d.set_item("tags", info.tags)?;
        d.set_item("created", info.created)?;
        d.set_item("modified", info.modified)?;
        d.set_item("unit", info.unit)?;
        d.set_item("elements", info.elements)?;
        Ok(d)
    }

    /// Copy `src` → `dst` keeping only sections named in `names` (byte-for-byte).
    #[staticmethod]
    fn split(src: &str, dst: &str, names: Vec<String>) -> PyResult<()> {
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        RsGeoData::split(src, dst, &refs).map_err(to_pyerr)
    }

    /// Copy `src` → `dst` keeping only sections tagged with any of `tags`.
    #[staticmethod]
    fn export(src: &str, dst: &str, tags: Vec<String>) -> PyResult<()> {
        let refs: Vec<&str> = tags.iter().map(String::as_str).collect();
        RsGeoData::export(src, dst, &refs).map_err(to_pyerr)
    }

    /// Merge projects `a` and `b` into `dst` (`b` wins on clash).
    #[staticmethod]
    fn merge(a: &str, b: &str, dst: &str) -> PyResult<()> {
        RsGeoData::merge(a, b, dst).map_err(to_pyerr)
    }

    /// The project owner recorded in the manifest, or `None`.
    #[getter]
    fn owner(&self) -> Option<String> {
        self.inner.owner().map(String::from)
    }
    fn set_owner(&mut self, owner: &str) {
        self.inner.set_owner(owner);
    }

    /// Project-level custom tags.
    #[getter]
    fn tags(&self) -> Vec<String> {
        self.inner.tags().to_vec()
    }
    fn set_tags(&mut self, tags: Vec<String>) {
        self.inner.set_tags(tags);
    }
    /// Tag a single element (by name) so `export(tags=...)` can select it.
    fn set_element_tags(&mut self, name: &str, tags: Vec<String>) {
        self.inner.set_element_tags(name, tags);
    }

    /// Store an opaque model section (petekSim's sidecar) — bytes petekIO frames
    /// and returns untouched, each with its own `version`.
    fn put_model_section(&mut self, name: &str, tags: Vec<String>, version: u32, data: &[u8]) {
        self.inner
            .put_model_section(name, tags, version, data.to_vec());
    }
    /// The names of the model sections held.
    fn model_section_names(&self) -> Vec<String> {
        self.inner.model_section_names()
    }
    /// A model section's `(version, bytes)`, or `None`.
    fn model_section<'py>(
        &self,
        py: Python<'py>,
        name: &str,
    ) -> Option<(u32, Bound<'py, PyBytes>)> {
        self.inner
            .model_section(name)
            .map(|(v, b)| (v, PyBytes::new(py, &b)))
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

    /// A tidy per-`zone × bore` table of `curve` across **every** well in this
    /// view, as a `pandas.DataFrame`. Like `Well.zone_table`, but `bore`
    /// identifies well + sidetrack (e.g. `"15/9-A1 B"`). `stats` default
    /// `["mean"]`; `zone` is an ordered Categorical in lithostratigraphic order;
    /// empty cells dropped unless `include_empty`. `pivot=True` → wide (`zone`
    /// index × `bore` columns; multi-stat → MultiIndex `(stat, bore)`).
    /// `aggregate=True` → grouped by zone with a pooled "all" row first
    /// (sample-weighted across wells), indexed by `(zone, bore)`; mutually
    /// exclusive with `pivot`. `zones` keeps only those zone names
    /// (case-insensitive). `weighted` (default True) thickness-weights the
    /// averages; `stats` may also be `samples`/`gross`. `decimals` rounds. pandas.
    #[pyo3(signature = (curve, stats=None, zones=None, include_empty=false, pivot=false, aggregate=false, weighted=true, decimals=None))]
    #[allow(clippy::too_many_arguments)]
    fn zone_table(
        &self,
        py: Python<'_>,
        curve: &str,
        stats: Option<Vec<String>>,
        zones: Option<Vec<String>>,
        include_empty: bool,
        pivot: bool,
        aggregate: bool,
        weighted: bool,
        decimals: Option<i64>,
    ) -> PyResult<Py<PyAny>> {
        let stats = stats.unwrap_or_else(|| vec!["mean".to_string()]);
        let g = self.geo.borrow(py);
        let mut bores: Vec<(String, &petekio::Sidetrack)> = Vec::new();
        for id in &self.ids {
            if let Some(w) = g.inner.well(id) {
                for s in w.sidetracks() {
                    let label = if s.label.is_empty() {
                        id.clone()
                    } else {
                        format!("{id} {}", s.label)
                    };
                    bores.push((label, s));
                }
            }
        }
        crate::well::build_zone_table(
            py,
            &bores,
            curve,
            &stats,
            zones.as_deref(),
            include_empty,
            pivot,
            aggregate,
            weighted,
            decimals,
        )
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
