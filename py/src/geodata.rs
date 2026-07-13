//! `GeoData` — the load-once project substrate: named surfaces, wells, points,
//! and polygons under one declared length unit. Mirrors `petekio::GeoData`.
//!
//! Surfaces are handed back as owned copies (deep-cloned); points/polygons/wells
//! are returned as lightweight **views** that re-resolve into this project's
//! collections by name. The wells view (`geo.wells()`) is the broadcast
//! substrate — see `well.rs`.

use crate::points::{PointSet, PolygonSet};
use crate::stats::Stats;
use crate::structured_surface::StructuredMeshSurface;
use crate::surface::Surface;
use crate::well::Well;
use crate::{parse_unit, to_pyerr, unit_label};
use petekio::GeoData as RsGeoData;
use pyo3::exceptions::{PyAttributeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyAny;
use pyo3::types::{PyBytes, PyDict, PyList};

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
    /// returning a cheap view (no grid copy).
    fn load_surface(slf: Bound<'_, Self>, name: &str, path: &str) -> PyResult<Surface> {
        slf.borrow_mut()
            .inner
            .load_surface(name, path)
            .map_err(to_pyerr)?;
        Ok(Surface::view(slf.clone().unbind(), name.to_string()))
    }

    /// Load an EarthVision grid into the shared surface namespace as a
    /// null-preserving structured mesh surface.
    fn load_structured_surface(
        slf: Bound<'_, Self>,
        name: &str,
        path: &str,
    ) -> PyResult<StructuredMeshSurface> {
        slf.borrow_mut()
            .inner
            .load_structured_surface(name, path)
            .map_err(to_pyerr)?;
        Ok(StructuredMeshSurface::view(
            slf.clone().unbind(),
            name.to_string(),
        ))
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

    /// Load an IRAP/RMS point set, using a matching EarthVision grid export for
    /// Petrel `column`/`row` topology.
    fn load_points_with_topology(
        slf: Bound<'_, Self>,
        name: &str,
        path: &str,
        topology_path: &str,
    ) -> PyResult<PointSet> {
        slf.borrow_mut()
            .inner
            .load_points_with_topology(name, path, topology_path)
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

    /// The surface stored under `name` (view), or `None`.
    fn surface(slf: Bound<'_, Self>, py: Python<'_>, name: &str) -> PyResult<Option<Py<PyAny>>> {
        let geo = slf.borrow();
        let regular = geo.inner.surface(name).is_some();
        let structured = geo.inner.structured_surface(name).is_some();
        drop(geo);
        if regular {
            return Ok(Some(
                Surface::view(slf.clone().unbind(), name.to_string())
                    .into_pyobject(py)?
                    .into_any()
                    .unbind(),
            ));
        }
        if structured {
            return Ok(Some(
                StructuredMeshSurface::view(slf.clone().unbind(), name.to_string())
                    .into_pyobject(py)?
                    .into_any()
                    .unbind(),
            ));
        }
        Ok(None)
    }

    /// Rename a stored surface.
    fn rename_surface(&mut self, old: &str, new: &str) -> PyResult<()> {
        self.inner.rename_surface(old, new).map_err(to_pyerr)
    }

    /// Delete a stored surface. Returns whether anything was removed.
    fn delete_surface(&mut self, name: &str) -> bool {
        self.inner.delete_surface(name)
    }

    /// The point set stored under `name` (view), or `None`.
    fn points(slf: Bound<'_, Self>, name: &str) -> Option<PointSet> {
        slf.borrow()
            .inner
            .points(name)
            .is_some()
            .then(|| PointSet::view(slf.clone().unbind(), name.to_string()))
    }

    /// Rename a stored point set.
    fn rename_points(&mut self, old: &str, new: &str) -> PyResult<()> {
        self.inner.rename_points(old, new).map_err(to_pyerr)
    }

    /// Delete a stored point set. Returns whether anything was removed.
    fn delete_points(&mut self, name: &str) -> bool {
        self.inner.delete_points(name)
    }

    /// The polygon set stored under `name` (view), or `None`.
    fn polygons(slf: Bound<'_, Self>, name: &str) -> Option<PolygonSet> {
        slf.borrow()
            .inner
            .polygons(name)
            .is_some()
            .then(|| PolygonSet::view(slf.clone().unbind(), name.to_string()))
    }

    /// Rename a stored polygon set.
    fn rename_polygons(&mut self, old: &str, new: &str) -> PyResult<()> {
        self.inner.rename_polygons(old, new).map_err(to_pyerr)
    }

    /// Delete a stored polygon set. Returns whether anything was removed.
    fn delete_polygons(&mut self, name: &str) -> bool {
        self.inner.delete_polygons(name)
    }

    /// Distinct persisted formation-top names across every well/bore.
    fn well_top_names(&self) -> Vec<String> {
        self.inner.well_top_names()
    }

    /// Persisted picks for one horizon as `(well, bore, md, xyz-or-None)` rows.
    #[allow(clippy::type_complexity)]
    fn well_top_set(&self, name: &str) -> Vec<(String, String, f64, Option<(f64, f64, f64)>)> {
        self.inner
            .well_top_set(name)
            .into_iter()
            .map(|row| (row.well, row.bore, row.md, row.xyz.map(|p| (p.x, p.y, p.z))))
            .collect()
    }

    /// Delete a persisted formation horizon globally; returns picks removed.
    fn delete_well_top(&mut self, name: &str) -> usize {
        self.inner.delete_well_top(name)
    }

    /// Load a well from `files` (a per-well directory or single file) under
    /// `id`, returning a view. `head` (wellhead `(x, y)`) and `kb` (datum) are
    /// **optional** — when a `.wellpath` is present its header is authoritative
    /// and overrides them, so for a positioned well you can omit both:
    /// `geo.load_well("15/9-A1", files="wells/")`. Without a `.wellpath` they
    /// default to `(0, 0)` / `0`.
    ///
    /// `ingest` (optional) is a declarative [`IngestSpec`] applied to **this
    /// load only** — curve `aliases` (canonicalize mnemonics: the map first, then
    /// the built-in table + vintage `_YYYY` strip) plus a declared `unit` guard
    /// (loud error if it disagrees with the project unit). It supersedes the
    /// sticky `aliases=` kwarg and the fluent `strat_hint(...)` mutation.
    ///
    /// `aliases` (**deprecated**, use `ingest=IngestSpec(aliases=...)`) sets a
    /// `{raw: canonical}` map as sticky project state affecting this and every
    /// subsequent `load_well`. Passing both `ingest` and `aliases` is a loud
    /// error.
    #[pyo3(signature = (id, head=None, kb=None, files=None, ingest=None, aliases=None))]
    fn load_well(
        slf: Bound<'_, Self>,
        id: &str,
        head: Option<(f64, f64)>,
        kb: Option<f64>,
        files: Option<&str>,
        ingest: Option<crate::specs::IngestSpec>,
        aliases: Option<std::collections::HashMap<String, String>>,
    ) -> PyResult<Well> {
        let files = files.ok_or_else(|| {
            PyValueError::new_err("load_well: `files` (a well directory or file) is required")
        })?;
        if ingest.is_some() && aliases.is_some() {
            return Err(PyValueError::new_err(
                "load_well: pass EITHER ingest=IngestSpec(...) OR the deprecated \
                 aliases= kwarg, not both",
            ));
        }
        let py = slf.py();
        let head = head.unwrap_or((0.0, 0.0));
        let kb = kb.unwrap_or(0.0);
        {
            let mut g = slf.borrow_mut();
            if let Some(spec) = &ingest {
                // Unit guard: a declared ingest unit must match the project unit.
                if let Some(u) = spec.unit_value() {
                    if u != g.inner.unit {
                        return Err(PyValueError::new_err(format!(
                            "load_well: IngestSpec unit '{}' disagrees with the project \
                             unit '{}' (well '{id}')",
                            unit_label(u),
                            unit_label(g.inner.unit),
                        )));
                    }
                }
                let aliases = spec.aliases_value().clone();
                let nm = (!aliases.is_empty()).then_some(aliases);
                g.inner
                    .load_well_with(id, head, kb, files, nm.as_ref())
                    .map_err(to_pyerr)?;
            } else {
                if let Some(map) = aliases {
                    crate::deprecation_warning(
                        py,
                        "load_well(aliases=...) is deprecated and mutates sticky project \
                         state; pass ingest=IngestSpec(aliases=...) instead (removal in a \
                         future minor).",
                    )?;
                    let nm = petekio::NameMap::from_pairs(map);
                    g.inner.set_curve_aliases(nm);
                }
                g.inner.load_well(id, head, kb, files).map_err(to_pyerr)?;
            }
        }
        Ok(Well::view(slf.clone().unbind(), id.to_string()))
    }

    /// Load a multi-well Petrel well-tops file; route each `Horizon` pick to the
    /// matching loaded well + bore. Returns the number of tops assigned. Also
    /// derives the project's global lithostratigraphic column (see
    /// `strat_order`) across every well in the file and pushes it into each
    /// loaded well, so `zones()`/`zone_stats()` present in that order.
    ///
    /// `ingest` (optional) is a declarative [`IngestSpec`]; its `strat_hints`
    /// (each `("above","below")` or `"A < B"`) are applied as soft ordering hints
    /// for this load — the declarative replacement for the fluent
    /// `strat_hint(...)` mutation. Names resolve at apply (a bad token errors
    /// loudly naming it).
    #[pyo3(signature = (path, ingest=None))]
    fn load_well_tops(
        &mut self,
        path: &str,
        ingest: Option<crate::specs::IngestSpec>,
    ) -> PyResult<usize> {
        if let Some(spec) = &ingest {
            self.inner.add_strat_hints(spec.strat_hints_value());
        }
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
    /// - shorthand: `geo.strat_hint("Base Shale < Upper Sand")` — `A < B` is "A
    ///   above B", `A > B` is "A below B"; sides may be partial names.
    /// - explicit: `geo.strat_hint(above="Base Shale top", below="Upper Sand top")`.
    ///
    /// **Deprecated** — this mutates sticky project state; pass the hints
    /// declaratively via `load_well_tops(ingest=IngestSpec(strat_hints=[...]))`
    /// instead (removal in a future minor).
    #[pyo3(signature = (spec=None, *, above=None, below=None))]
    fn strat_hint(
        &mut self,
        py: Python<'_>,
        spec: Option<&str>,
        above: Option<&str>,
        below: Option<&str>,
    ) -> PyResult<()> {
        crate::deprecation_warning(
            py,
            "GeoData.strat_hint(...) is deprecated and mutates sticky project state; \
             pass load_well_tops(ingest=IngestSpec(strat_hints=[...])) instead (removal \
             in a future minor).",
        )?;
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

    /// Rename a stored well.
    fn rename_well(&mut self, old: &str, new: &str) -> PyResult<()> {
        self.inner.rename_well(old, new).map_err(to_pyerr)
    }

    /// Delete a stored well. Returns whether anything was removed.
    fn delete_well(&mut self, id: &str) -> bool {
        self.inner.delete_well(id)
    }

    /// All surfaces in insertion order, as cheap views (no grid copy).
    fn surfaces(slf: Bound<'_, Self>, py: Python<'_>) -> PyResult<Vec<Py<PyAny>>> {
        let geo = slf.borrow();
        let regular: Vec<String> = geo
            .inner
            .surfaces_named()
            .map(|(name, _)| name.to_string())
            .collect();
        let structured: Vec<String> = geo
            .inner
            .structured_surfaces_named()
            .map(|(name, _)| name.to_string())
            .collect();
        drop(geo);
        let mut out = Vec::with_capacity(regular.len() + structured.len());
        for name in regular {
            out.push(
                Surface::view(slf.clone().unbind(), name)
                    .into_pyobject(py)?
                    .into_any()
                    .unbind(),
            );
        }
        for name in structured {
            out.push(
                StructuredMeshSurface::view(slf.clone().unbind(), name)
                    .into_pyobject(py)?
                    .into_any()
                    .unbind(),
            );
        }
        Ok(out)
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
    fn save(&self, py: Python<'_>, path: &str) -> PyResult<()> {
        py.detach(|| self.inner.save(path)).map_err(to_pyerr)
    }

    /// Open a `.pproj` project.
    #[staticmethod]
    fn open(py: Python<'_>, path: &str) -> PyResult<GeoData> {
        Ok(GeoData {
            inner: py.detach(|| RsGeoData::open(path)).map_err(to_pyerr)?,
        })
    }

    /// Read a project's manifest (owner/tags/unit/timestamps + element index)
    /// without decoding any element — a dict.
    #[staticmethod]
    fn inspect<'py>(py: Python<'py>, path: &str) -> PyResult<Bound<'py, PyDict>> {
        let info = py.detach(|| RsGeoData::inspect(path)).map_err(to_pyerr)?;
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
    fn split(py: Python<'_>, src: &str, dst: &str, names: Vec<String>) -> PyResult<()> {
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        py.detach(|| RsGeoData::split(src, dst, &refs))
            .map_err(to_pyerr)
    }

    /// Copy `src` → `dst` keeping only sections tagged with any of `tags`.
    #[staticmethod]
    fn export(py: Python<'_>, src: &str, dst: &str, tags: Vec<String>) -> PyResult<()> {
        let refs: Vec<&str> = tags.iter().map(String::as_str).collect();
        py.detach(|| RsGeoData::export(src, dst, &refs))
            .map_err(to_pyerr)
    }

    /// Merge projects `a` and `b` into `dst` (`b` wins on clash).
    #[staticmethod]
    fn merge(py: Python<'_>, a: &str, b: &str, dst: &str) -> PyResult<()> {
        py.detach(|| RsGeoData::merge(a, b, dst)).map_err(to_pyerr)
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

    fn evaluate_intersections(
        &self,
        py: Python<'_>,
        surface: &Bound<'_, PyAny>,
        tolerance: f64,
        all: bool,
    ) -> PyResult<crate::intersection::WellIntersectionSet> {
        let full_scope = {
            let geo = self.geo.borrow(py);
            let all_ids: Vec<&str> = geo.inner.wells_named().map(|(id, _)| id).collect();
            all_ids.len() == self.ids.len()
                && all_ids
                    .iter()
                    .zip(&self.ids)
                    .all(|(actual, selected)| *actual == selected)
        };
        let (mut result, surface_name) = crate::intersection::with_surface(py, surface, |s| {
            let geo = self.geo.borrow(py);
            let view = geo.inner.wells_by_ids(&self.ids);
            if all {
                view.intersections(s, tolerance)
            } else {
                view.intersection(s, tolerance)
            }
        })?;
        for hit in &mut result.hits {
            hit.surface = surface_name.clone();
        }
        Ok(crate::intersection::WellIntersectionSet::new(
            result,
            self.geo.clone_ref(py),
            full_scope,
        ))
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

    /// One hit per bore across the view; no-hit and failed bores remain in the
    /// returned diagnostics rather than aborting other wells.
    #[pyo3(signature = (surface, tolerance=1e-3))]
    fn intersection(
        &self,
        py: Python<'_>,
        surface: &Bound<'_, PyAny>,
        tolerance: f64,
    ) -> PyResult<crate::intersection::WellIntersectionSet> {
        self.evaluate_intersections(py, surface, tolerance, false)
    }

    /// All crossings per bore across the view.
    #[pyo3(signature = (surface, tolerance=1e-3))]
    fn intersections(
        &self,
        py: Python<'_>,
        surface: &Bound<'_, PyAny>,
        tolerance: f64,
    ) -> PyResult<crate::intersection::WellIntersectionSet> {
        self.evaluate_intersections(py, surface, tolerance, true)
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
    /// averages; `stats` may also be `samples`/`gross`. `decimals` rounds. Pass
    /// `cut=NetSettings(...)` to net-condition every cell first (see
    /// `Well.zone_table`); `phi`/`sw`/`vsh` name the conditioning curves. pandas.
    #[pyo3(signature = (curve, stats=None, zones=None, include_empty=false, pivot=false, aggregate=false, weighted=true, decimals=None, cut=None, phi="PHIE", sw="SW", vsh=None))]
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
        cut: Option<crate::specs::NetSettings>,
        phi: &str,
        sw: &str,
        vsh: Option<&str>,
    ) -> PyResult<Py<PyAny>> {
        let stats = stats.unwrap_or_else(|| vec!["mean".to_string()]);
        let net_cut = cut.map(|c| c.cutoffs());
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
            net_cut,
            phi,
            sw,
            vsh,
        )
    }

    /// Open a standalone **logs-only** viewer session over every well in this
    /// view — a multi-well correlation panel, the producer slice of the
    /// well-correlation seam
    /// (`petekSuite/dev-docs/designs/well-log-bundle-seam.md`). Builds one
    /// `WellLogBundle` (`kind "wells_logs"`, schema v4) from the wells' own logs
    /// + trajectories (no model) and hands it to the viewer unit
    /// (`petektools.viewer`, an optional runtime dependency imported lazily).
    /// Arguments mirror `Well.view` (`spec=ViewSpec`/`settings=ViewSettings` or
    /// the legacy per-call kwargs; spec XOR kwargs, loud on both). Returns the
    /// `LogSession`.
    #[pyo3(signature = (spec=None, settings=None, curves=None, tops=None, flatten_default=None, phie_cutoff=None, flags=None, serve=None, save=None))]
    #[allow(clippy::too_many_arguments)]
    fn view(
        &self,
        py: Python<'_>,
        spec: Option<Py<PyAny>>,
        settings: Option<Py<PyAny>>,
        curves: Option<Vec<String>>,
        tops: Option<Py<PyAny>>,
        flatten_default: Option<String>,
        phie_cutoff: Option<f64>,
        flags: Option<Vec<String>>,
        serve: Option<bool>,
        save: Option<String>,
    ) -> PyResult<Py<PyAny>> {
        let what_set = curves.is_some()
            || tops.is_some()
            || flatten_default.is_some()
            || phie_cutoff.is_some()
            || flags.is_some();
        let how_set = serve.is_some() || save.is_some();
        crate::viewer::view_xor_guard(spec.is_some(), settings.is_some(), what_set, how_set)?;
        let gather = crate::viewer::gather_curves(py, spec.as_ref(), curves)?;
        let list = PyList::empty(py);
        {
            let g = self.geo.borrow(py);
            for id in &self.ids {
                if let Some(w) = g.inner.well(id) {
                    let raw = crate::viewer::raw_log_well(py, w, gather.as_deref())?;
                    list.append(raw)?;
                }
            }
        }
        crate::viewer::render(
            py,
            list.into_any(),
            spec,
            settings,
            tops,
            flatten_default,
            phie_cutoff,
            flags,
            serve,
            save,
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
