//! `Well` / `Interval` / `LogView` — the well-geometry + log access chain, with
//! the dynamic `w.brent.ntg` `__getattr__` ergonomic. Mirrors `petekio::{Well,
//! Interval, LogView}`.
//!
//! These are all **views** into a `GeoData` project: a `Well` holds the owning
//! project plus a well id and re-resolves the borrowed `&Well` on each call
//! (Rust's `Interval`/`LogView` carry lifetimes and cannot be stored in a
//! `#[pyclass]`, so the binding stores the identifying keys and resolves lazily).
//! Curve samples come back as plain `list[float]` — numpy is out of scope.

use crate::geodata::GeoData;
use crate::specs::NetSettings;
use crate::stats::Stats;
use petekio::{Log as RsLog, Well as RsWell};
use pyo3::exceptions::{PyAttributeError, PyImportError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyAny, PyDict};

/// The `Stats` attribute names a `zone_table` `stats=` may request.
pub(crate) const STAT_ATTRS: &[&str] = &[
    "mean", "sum", "count", "min", "max", "std", "p10", "p50", "p90",
];

/// Extra `stats=` options not on `Stats`: `samples` (= sample count) and `gross`
/// (the zone's MD thickness — geometry, not from the curve).
pub(crate) const EXTRA_STATS: &[&str] = &["samples", "gross"];

/// Build a per-`zone × bore` `pandas.DataFrame` for `curve` over `bores`. Thin
/// shim: validates the arg contract, runs the pooling/aggregation crunch in core
/// (`petekio::analysis::well_tables::build_zone_table`, off the GIL), then
/// marshals the returned columns into pandas. `zone` is set as an **ordered
/// Categorical** in lithostratigraphic order so it survives `pivot`/`groupby`;
/// `pivot` reshapes wide (mutually exclusive with `aggregate`); `decimals` rounds.
/// pandas is imported lazily.
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_zone_table(
    py: Python<'_>,
    bores: &[(String, &petekio::Sidetrack)],
    curve: &str,
    stats: &[String],
    zones: Option<&[String]>,
    include_empty: bool,
    pivot: bool,
    aggregate: bool,
    weighted: bool,
    decimals: Option<i64>,
    net_cut: Option<petekio::Cutoffs>,
    net_phi: &str,
    net_sw: &str,
    net_vsh: Option<&str>,
) -> PyResult<Py<PyAny>> {
    use petekio::analysis::well_tables::{build_zone_table as core_build, NetCond, ZoneTable};
    for s in stats {
        if !STAT_ATTRS.contains(&s.as_str()) && !EXTRA_STATS.contains(&s.as_str()) {
            return Err(PyValueError::new_err(format!(
                "zone_table: unknown stat '{s}' (expected one of: {}, {})",
                STAT_ATTRS.join(", "),
                EXTRA_STATS.join(", ")
            )));
        }
    }
    if pivot && aggregate {
        return Err(PyValueError::new_err(
            "zone_table: use either pivot=True or aggregate=True, not both",
        ));
    }
    let stat_refs: Vec<&str> = stats.iter().map(String::as_str).collect();
    // The crunch is pure Rust — run it with the GIL released.
    let table = py.detach(|| {
        let net = net_cut.map(|cut| NetCond {
            cut,
            phi: net_phi,
            sw: net_sw,
            vsh: net_vsh,
        });
        core_build(
            bores,
            curve,
            &stat_refs,
            zones,
            include_empty,
            aggregate,
            weighted,
            net,
        )
    });

    let pd = py
        .import("pandas")
        .map_err(|_| PyImportError::new_err("zone_table requires pandas — `pip install pandas`"))?;

    // Fill a `{zone, bore, <stat>...}` dict for a DataFrame; `zone_obj` is either
    // the raw string column or a prepared ordered Categorical.
    let build_data = |zone_obj: Bound<'_, PyAny>,
                      bore: Vec<String>,
                      cols: &[Vec<f64>]|
     -> PyResult<Bound<'_, PyDict>> {
        let data = PyDict::new(py);
        data.set_item("zone", zone_obj)?;
        data.set_item("bore", bore)?;
        for (name, col) in stats.iter().zip(cols) {
            data.set_item(name.as_str(), col.clone())?;
        }
        Ok(data)
    };

    match table {
        // aggregate=True → grouped, indexed by (zone, bore).
        ZoneTable::Aggregate { zone, bore, cols } => {
            let data = build_data(zone.into_pyobject(py)?.into_any(), bore, &cols)?;
            let mut df = pd.call_method1("DataFrame", (data,))?;
            df = df.call_method1("set_index", (vec!["zone", "bore"],))?;
            if let Some(d) = decimals {
                df = df.call_method1("round", (d,))?;
            }
            Ok(df.unbind())
        }
        // Flat tidy / pivot. `zone` is an ordered Categorical (built directly as
        // the column, not reassigned — that would trip pandas' copy-on-write).
        ZoneTable::Tidy {
            zone,
            bore,
            cols,
            categories,
        } => {
            let kwargs = PyDict::new(py);
            kwargs.set_item("categories", categories)?;
            kwargs.set_item("ordered", true)?;
            let zone_cat = pd.call_method("Categorical", (zone,), Some(&kwargs))?;
            let data = build_data(zone_cat, bore, &cols)?;
            let mut df = pd.call_method1("DataFrame", (data,))?;
            if pivot {
                let kwargs = PyDict::new(py);
                kwargs.set_item("index", "zone")?;
                kwargs.set_item("columns", "bore")?;
                if stats.len() == 1 {
                    kwargs.set_item("values", stats[0].as_str())?;
                } else {
                    kwargs.set_item("values", stats.to_vec())?;
                }
                df = df.call_method("pivot", (), Some(&kwargs))?;
            }
            if let Some(d) = decimals {
                df = df.call_method1("round", (d,))?;
            }
            Ok(df.unbind())
        }
    }
}

/// A well: a view into a `GeoData` project's well collection.
#[pyclass(name = "Well")]
pub struct Well {
    geo: Py<GeoData>,
    id: String,
}

impl Well {
    pub(crate) fn view(geo: Py<GeoData>, id: String) -> Well {
        Well { geo, id }
    }

    /// Resolve the borrowed Rust well and run `f` over it.
    pub(crate) fn with_well<R>(
        &self,
        py: Python<'_>,
        f: impl FnOnce(&RsWell) -> PyResult<R>,
    ) -> PyResult<R> {
        let g = self.geo.borrow(py);
        let w = g
            .inner
            .well(&self.id)
            .ok_or_else(|| PyValueError::new_err(format!("no well '{}'", self.id)))?;
        f(w)
    }

    /// Guard a top-level (bore-picking) accessor: on a **multi-bore** well with no
    /// [default bore](Well::set_default_bore) set, raise rather than silently
    /// resolve through the empty main bore (the R-a "silent-empty" bug). Names the
    /// bores and points at `.sidetrack(name)` / `.set_default_bore(name)`.
    fn require_resolvable(&self, py: Python<'_>) -> PyResult<()> {
        self.with_well(py, |w| {
            if w.is_multibore() && w.default_bore().is_none() {
                let bores: Vec<&str> = w.bores().filter(|b| !b.is_empty()).collect();
                Err(PyValueError::new_err(format!(
                    "well '{}' has {} bores ({}) — a top-level accessor can't pick one; \
                     use .sidetrack(name) for a specific bore or .set_default_bore(name)",
                    self.id,
                    bores.len(),
                    bores.join(", ")
                )))
            } else {
                Ok(())
            }
        })
    }
}

#[pymethods]
impl Well {
    #[getter]
    fn id(&self) -> &str {
        &self.id
    }

    #[getter]
    fn head(&self, py: Python<'_>) -> PyResult<(f64, f64)> {
        self.with_well(py, |w| Ok(w.head))
    }

    #[getter]
    fn kb(&self, py: Python<'_>) -> PyResult<f64> {
        self.with_well(py, |w| Ok(w.kb))
    }

    /// Interpolated position `(x, y, z)` at measured depth `md`, or `None`.
    /// Raises on a multi-bore well with no default bore (select one first).
    fn xyz(&self, py: Python<'_>, md: f64) -> PyResult<Option<(f64, f64, f64)>> {
        self.require_resolvable(py)?;
        self.with_well(py, |w| Ok(w.xyz(md).map(|p| (p.x, p.y, p.z))))
    }

    /// All crossings of the resolved bore with `surface`, MD ordered.
    #[pyo3(signature = (surface, tolerance=1e-3))]
    fn intersections(
        &self,
        py: Python<'_>,
        surface: &Bound<'_, PyAny>,
        tolerance: f64,
    ) -> PyResult<Vec<crate::intersection::SurfaceIntersection>> {
        self.require_resolvable(py)?;
        let (hits, surface_name) = crate::intersection::with_surface(py, surface, |s| {
            self.with_well(py, |well| {
                well.intersections(s, tolerance).map_err(crate::to_pyerr)
            })
            .map_err(|error| petekio::GeoError::Parse(error.to_string()))
        })?;
        Ok(hits
            .into_iter()
            .map(|hit| {
                crate::intersection::SurfaceIntersection::attach_project(
                    hit,
                    surface_name.as_deref(),
                    self.geo.as_ptr() as usize,
                )
            })
            .collect())
    }

    /// The sole crossing of the resolved bore, or `None`.
    #[pyo3(signature = (surface, tolerance=1e-3))]
    fn intersection(
        &self,
        py: Python<'_>,
        surface: &Bound<'_, PyAny>,
        tolerance: f64,
    ) -> PyResult<Option<crate::intersection::SurfaceIntersection>> {
        self.require_resolvable(py)?;
        let (hit, surface_name) = crate::intersection::with_surface(py, surface, |s| {
            self.with_well(py, |well| {
                well.intersection(s, tolerance).map_err(crate::to_pyerr)
            })
            .map_err(|error| petekio::GeoError::Parse(error.to_string()))
        })?;
        Ok(hit.map(|hit| {
            crate::intersection::SurfaceIntersection::attach_project(
                hit,
                surface_name.as_deref(),
                self.geo.as_ptr() as usize,
            )
        }))
    }

    /// Formation-top records on the resolved bore as `(name, md)` rows.
    fn tops(&self, py: Python<'_>) -> PyResult<Vec<(String, f64)>> {
        self.require_resolvable(py)?;
        self.with_well(py, |well| {
            Ok(well.tops().map(|top| (top.name.clone(), top.md)).collect())
        })
    }

    /// Add a top from a finite MD or a matching `SurfaceIntersection`.
    fn add_top(&self, py: Python<'_>, name: &str, md_or_hit: &Bound<'_, PyAny>) -> PyResult<()> {
        self.require_resolvable(py)?;
        let value = top_value(md_or_hit)?;
        validate_top_project(&value, self.geo.as_ptr() as usize)?;
        let mut geo = self.geo.borrow_mut(py);
        let well = geo
            .inner
            .well_mut(&self.id)
            .ok_or_else(|| PyValueError::new_err(format!("no well '{}'", self.id)))?;
        match value {
            TopValue::Md(md) => well.add_top(name, md),
            TopValue::Hit(hit, _) => well.add_top_from_intersection(name, &hit),
        }
        .map_err(crate::to_pyerr)
    }

    /// Replace an existing top from a finite MD or matching intersection.
    fn replace_top(
        &self,
        py: Python<'_>,
        name: &str,
        md_or_hit: &Bound<'_, PyAny>,
    ) -> PyResult<()> {
        self.require_resolvable(py)?;
        let value = top_value(md_or_hit)?;
        validate_top_project(&value, self.geo.as_ptr() as usize)?;
        let mut geo = self.geo.borrow_mut(py);
        let well = geo
            .inner
            .well_mut(&self.id)
            .ok_or_else(|| PyValueError::new_err(format!("no well '{}'", self.id)))?;
        match value {
            TopValue::Md(md) => well.replace_top(name, md),
            TopValue::Hit(hit, _) => well.replace_top_from_intersection(name, &hit),
        }
        .map_err(crate::to_pyerr)
    }

    /// Remove a named top from the resolved bore.
    fn remove_top(&self, py: Python<'_>, name: &str) -> PyResult<()> {
        self.require_resolvable(py)?;
        let mut geo = self.geo.borrow_mut(py);
        geo.inner
            .well_mut(&self.id)
            .ok_or_else(|| PyValueError::new_err(format!("no well '{}'", self.id)))?
            .remove_top(name)
            .map(|_| ())
            .map_err(crate::to_pyerr)
    }

    /// TVD at measured depth `md`, or `None`.
    /// Raises on a multi-bore well with no default bore (select one first).
    fn tvd(&self, py: Python<'_>, md: f64) -> PyResult<Option<f64>> {
        self.require_resolvable(py)?;
        self.with_well(py, |w| Ok(w.tvd(md)))
    }

    /// Measured depth at a given TVD, or `None`.
    /// Raises on a multi-bore well with no default bore (select one first).
    fn md_at_tvd(&self, py: Python<'_>, tvd: f64) -> PyResult<Option<f64>> {
        self.require_resolvable(py)?;
        self.with_well(py, |w| Ok(w.md_at_tvd(tvd)))
    }

    /// Whether the well is multi-bore (more than one bore carries a trajectory).
    #[getter]
    fn is_multibore(&self, py: Python<'_>) -> PyResult<bool> {
        self.with_well(py, |w| Ok(w.is_multibore()))
    }

    /// The explicitly selected default bore label, or `None`.
    #[getter]
    fn default_bore(&self, py: Python<'_>) -> PyResult<Option<String>> {
        self.with_well(py, |w| Ok(w.default_bore().map(String::from)))
    }

    /// Select the bore the top-level accessors resolve through (`""` = main bore).
    /// Raises `ValueError` if no such bore exists. Overrides the single-trajectory
    /// rule; the natural way to work a multi-bore well through the well-level API.
    fn set_default_bore(&self, py: Python<'_>, label: &str) -> PyResult<()> {
        let mut g = self.geo.borrow_mut(py);
        let w = g
            .inner
            .well_mut(&self.id)
            .ok_or_else(|| PyValueError::new_err(format!("no well '{}'", self.id)))?;
        w.set_default_bore(label)
            .map_err(|e| PyValueError::new_err(e.to_string()))
    }

    /// The interval named by top `name` (case-insensitive), or `None`.
    /// Raises on a multi-bore well with no default bore (select one first).
    fn top(slf: Bound<'_, Self>, name: &str) -> PyResult<Option<Interval>> {
        let py = slf.py();
        slf.borrow().require_resolvable(py)?;
        let exists = slf.borrow().with_well(py, |w| Ok(w.top(name).is_some()))?;
        Ok(exists.then(|| Interval {
            well: slf.clone().unbind(),
            top_name: name.to_string(),
        }))
    }

    /// A full-curve view of log `mnemonic` (case-insensitive), or `None`.
    /// Raises on a multi-bore well with no default bore (select one first).
    fn log(slf: Bound<'_, Self>, mnemonic: &str) -> PyResult<Option<LogView>> {
        let py = slf.py();
        slf.borrow().require_resolvable(py)?;
        let exists = slf
            .borrow()
            .with_well(py, |w| Ok(w.log(mnemonic).is_some()))?;
        Ok(exists.then(|| LogView {
            well: slf.clone().unbind(),
            mnemonic: mnemonic.to_string(),
            top_name: None,
            bore: None,
        }))
    }

    /// The coordinate reference system label, or `None`.
    #[getter]
    fn crs(&self, py: Python<'_>) -> PyResult<Option<String>> {
        self.with_well(py, |w| Ok(w.crs().map(|s| s.to_string())))
    }

    /// Fluid-contact picks on the resolved bore as `(name, md)` rows.
    /// Raises on a multi-bore well with no default bore (select one first).
    fn contacts(&self, py: Python<'_>) -> PyResult<Vec<(String, f64)>> {
        self.require_resolvable(py)?;
        self.with_well(py, |w| {
            Ok(w.contacts().map(|c| (c.name.clone(), c.md)).collect())
        })
    }

    /// The named fluid-contact pick on the resolved bore as `(name, md)`, or `None`.
    /// Raises on a multi-bore well with no default bore (select one first).
    fn contact(&self, py: Python<'_>, name: &str) -> PyResult<Option<(String, f64)>> {
        self.require_resolvable(py)?;
        self.with_well(py, |w| Ok(w.contact(name).map(|c| (c.name.clone(), c.md))))
    }

    /// The bore (sidetrack) labels, in order (`""` is the main bore).
    fn bores(&self, py: Python<'_>) -> PyResult<Vec<String>> {
        self.with_well(py, |w| {
            Ok(w.sidetracks().map(|s| s.label.clone()).collect())
        })
    }

    /// The bore with `label`, or `None`.
    fn sidetrack(slf: Bound<'_, Self>, label: &str) -> PyResult<Option<Sidetrack>> {
        let py = slf.py();
        let exists = slf
            .borrow()
            .with_well(py, |w| Ok(w.sidetrack(label).is_some()))?;
        Ok(exists.then(|| Sidetrack {
            well: slf.clone().unbind(),
            label: label.to_string(),
        }))
    }

    /// A tidy per-`zone × bore` table of `curve` across this well's bores, as a
    /// `pandas.DataFrame` (columns `zone`, `bore`, then each requested stat).
    /// `stats` are `Stats` attribute names (default `["mean"]`; any of
    /// mean/sum/count/min/max/std/p10/p50/p90). `zone` is an ordered Categorical
    /// in lithostratigraphic order, so it survives `pivot`/`groupby`. Empty
    /// (zero-thickness / no-sample) cells are dropped unless `include_empty`.
    /// `pivot=True` returns wide instead: `zone` index × `bore` columns (one stat
    /// → flat; several → MultiIndex `(stat, bore)`). `aggregate=True` groups by
    /// zone with a pooled "all" row first (sample-weighted across bores), indexed
    /// by `(zone, bore)`; mutually exclusive with `pivot`. `decimals` rounds the
    /// stat values. `zones` keeps only those zone names (case-insensitive).
    /// `weighted` (default True) thickness-weights the averages by each sample's
    /// MD span (so mixed sampling rates don't bias); `stats` may also be
    /// `samples`/`gross`. Pass `cut=NetSettings(...)` to **net-condition** every
    /// cell first (only samples passing the φ/Sw[/Vsh] cutoffs are pooled), with
    /// the conditioning curves named by `phi`/`sw`/`vsh` (defaults `PHIE`/`SW`/
    /// none — inert without `cut`). Requires pandas.
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
        cut: Option<NetSettings>,
        phi: &str,
        sw: &str,
        vsh: Option<&str>,
    ) -> PyResult<Py<PyAny>> {
        let stats = stats.unwrap_or_else(|| vec!["mean".to_string()]);
        let net_cut = cut.map(|c| c.cutoffs());
        self.with_well(py, |w| {
            let bores: Vec<(String, &petekio::Sidetrack)> =
                w.sidetracks().map(|s| (s.label.clone(), s)).collect();
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
        })
    }

    /// Open a standalone logs viewer session for this well — the producer slice
    /// of the well-correlation seam
    /// (`petekSuite/dev-docs/designs/well-log-bundle-seam.md`). Builds a
    /// `WellLogBundle` (`kind "wells_logs"`, schema v4) straight from this well's
    /// logs + trajectory (no model) and hands it to the viewer unit
    /// (`petektools.viewer`, an optional runtime dependency imported lazily).
    ///
    /// A `ViewSpec` (`spec=`) declares WHAT to show — `curves`/`tops`/
    /// `flatten_default`/`flags`/`cutoff` — and a `ViewSettings` (`settings=`)
    /// HOW to deliver it (`serve`/`save`). The legacy per-call kwargs remain as a
    /// convenience: `curves` selects mnemonics (canonical or raw; default all);
    /// `tops` opts picks/zones in (`True`/list/omitted); `flatten_default` presets
    /// the flatten pick; `phie_cutoff` (default 0.08) draws the PHIE net cutoff
    /// line; `flags` names extra categorical strips; `serve` (default True)
    /// serves non-blocking, `save="out.html"` exports instead. Passing a `spec`
    /// with any legacy WHAT kwarg (or `settings` with a HOW kwarg) is a loud
    /// error. Returns the `LogSession` (`.serve()` / `.save(path)` / `.bundle()`).
    #[pyo3(name = "view", signature = (spec=None, settings=None, curves=None, tops=None, flatten_default=None, phie_cutoff=None, flags=None, serve=None, save=None))]
    #[allow(clippy::too_many_arguments)]
    fn view_session(
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
        let raw = self.with_well(py, |w| {
            crate::viewer::raw_log_well(py, w, gather.as_deref())
        })?;
        let data = raw.into_bound(py).into_any();
        crate::viewer::render(
            py,
            data,
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

    /// All bores (sidetracks), in order.
    fn sidetracks(slf: Bound<'_, Self>) -> PyResult<Vec<Sidetrack>> {
        let py = slf.py();
        let labels = slf.borrow().with_well(py, |w| {
            Ok(w.sidetracks().map(|s| s.label.clone()).collect::<Vec<_>>())
        })?;
        Ok(labels
            .into_iter()
            .map(|label| Sidetrack {
                well: slf.clone().unbind(),
                label,
            })
            .collect())
    }

    /// Dynamic top access: `w.brent` → the `Brent` `Interval`. Falls back to a
    /// normal `AttributeError` for unknown names.
    fn __getattr__(slf: Bound<'_, Self>, name: String) -> PyResult<Interval> {
        if name.starts_with('_') {
            return Err(PyAttributeError::new_err(name));
        }
        let py = slf.py();
        let exists = slf.borrow().with_well(py, |w| Ok(w.top(&name).is_some()))?;
        if exists {
            Ok(Interval {
                well: slf.clone().unbind(),
                top_name: name,
            })
        } else {
            Err(PyAttributeError::new_err(format!(
                "'Well' object has no attribute or top '{name}'"
            )))
        }
    }

    fn __repr__(&self) -> String {
        format!("Well(id={:?})", self.id)
    }
}

/// A bore (sidetrack): a view resolving `well.sidetrack(label)`. Exposes
/// per-bore logs, zones, and aggregates (the real data lives on the named bores
/// A/B/C/ST2, not the main bore).
#[pyclass(name = "Sidetrack")]
pub struct Sidetrack {
    well: Py<Well>,
    label: String,
}

impl Sidetrack {
    fn resolve<R>(
        &self,
        py: Python<'_>,
        f: impl FnOnce(&petekio::Sidetrack) -> PyResult<R>,
    ) -> PyResult<R> {
        let w = self.well.borrow(py);
        w.with_well(py, |rw| {
            let st = rw
                .sidetrack(&self.label)
                .ok_or_else(|| PyValueError::new_err(format!("no bore '{}'", self.label)))?;
            f(st)
        })
    }

    fn mutate<R>(
        &self,
        py: Python<'_>,
        f: impl FnOnce(&mut petekio::Sidetrack) -> PyResult<R>,
    ) -> PyResult<R> {
        let well = self.well.borrow(py);
        let mut geo = well.geo.borrow_mut(py);
        let rw = geo
            .inner
            .well_mut(&well.id)
            .ok_or_else(|| PyValueError::new_err(format!("no well '{}'", well.id)))?;
        let st = rw.sidetrack_mut(&self.label);
        f(st)
    }
}

#[pymethods]
impl Sidetrack {
    #[getter]
    fn label(&self) -> &str {
        &self.label
    }

    /// The mnemonics of every curve on this bore, in insertion order.
    fn mnemonics(&self, py: Python<'_>) -> PyResult<Vec<String>> {
        self.resolve(py, |s| Ok(s.logs().map(|l| l.mnemonic.clone()).collect()))
    }

    /// TVD at measured depth `md` on this bore, or `None`.
    fn tvd(&self, py: Python<'_>, md: f64) -> PyResult<Option<f64>> {
        self.resolve(py, |s| Ok(s.tvd(md)))
    }

    /// Interpolated position `(x, y, z)` at `md`, or `None`.
    fn xyz(&self, py: Python<'_>, md: f64) -> PyResult<Option<(f64, f64, f64)>> {
        self.resolve(py, |s| Ok(s.xyz(md).map(|p| (p.x, p.y, p.z))))
    }

    /// All crossings of this bore with `surface`, MD ordered.
    #[pyo3(signature = (surface, tolerance=1e-3))]
    fn intersections(
        &self,
        py: Python<'_>,
        surface: &Bound<'_, PyAny>,
        tolerance: f64,
    ) -> PyResult<Vec<crate::intersection::SurfaceIntersection>> {
        let well_id = self.well.borrow(py).id.clone();
        let (mut hits, surface_name) = crate::intersection::with_surface(py, surface, |s| {
            self.resolve(py, |bore| {
                bore.intersections(s, tolerance).map_err(crate::to_pyerr)
            })
            .map_err(|error| petekio::GeoError::Parse(error.to_string()))
        })?;
        for hit in &mut hits {
            hit.well = Some(well_id.clone());
            hit.surface = surface_name.clone();
        }
        Ok(hits
            .into_iter()
            .map(|hit| {
                crate::intersection::SurfaceIntersection::attach_project(
                    hit,
                    surface_name.as_deref(),
                    self.well.borrow(py).geo.as_ptr() as usize,
                )
            })
            .collect())
    }

    /// The sole crossing of this bore, or `None`.
    #[pyo3(signature = (surface, tolerance=1e-3))]
    fn intersection(
        &self,
        py: Python<'_>,
        surface: &Bound<'_, PyAny>,
        tolerance: f64,
    ) -> PyResult<Option<crate::intersection::SurfaceIntersection>> {
        let hits = self.intersections(py, surface, tolerance)?;
        match hits.len() {
            0 => Ok(None),
            1 => Ok(hits.into_iter().next()),
            n => Err(PyValueError::new_err(format!(
                "trajectory crosses the surface {n} times; call intersections(...) and select a crossing explicitly"
            ))),
        }
    }

    /// Formation tops on this bore as `(name, md)` rows.
    fn tops(&self, py: Python<'_>) -> PyResult<Vec<(String, f64)>> {
        self.resolve(py, |bore| {
            Ok(bore.tops().map(|top| (top.name.clone(), top.md)).collect())
        })
    }

    fn add_top(&self, py: Python<'_>, name: &str, md_or_hit: &Bound<'_, PyAny>) -> PyResult<()> {
        let value = top_value(md_or_hit)?;
        validate_top_project(&value, self.well.borrow(py).geo.as_ptr() as usize)?;
        self.mutate(py, |bore| {
            match value {
                TopValue::Md(md) => bore.add_top(name, md),
                TopValue::Hit(hit, _) => bore.add_top_from_intersection(name, &hit),
            }
            .map_err(crate::to_pyerr)
        })
    }

    fn replace_top(
        &self,
        py: Python<'_>,
        name: &str,
        md_or_hit: &Bound<'_, PyAny>,
    ) -> PyResult<()> {
        let value = top_value(md_or_hit)?;
        validate_top_project(&value, self.well.borrow(py).geo.as_ptr() as usize)?;
        self.mutate(py, |bore| {
            match value {
                TopValue::Md(md) => bore.replace_top(name, md),
                TopValue::Hit(hit, _) => bore.replace_top_from_intersection(name, &hit),
            }
            .map_err(crate::to_pyerr)
        })
    }

    fn remove_top(&self, py: Python<'_>, name: &str) -> PyResult<()> {
        self.mutate(py, |bore| {
            bore.remove_top(name).map(|_| ()).map_err(crate::to_pyerr)
        })
    }

    /// The `(min, max)` measured-depth span of the active trajectory, or `None`.
    fn md_range(&self, py: Python<'_>) -> PyResult<Option<(f64, f64)>> {
        self.resolve(py, |s| {
            Ok((!s.trajectories().is_empty()).then(|| s.active().md_range()))
        })
    }

    /// Full-curve NaN-skipping `Stats` for `mnemonic` (case-insensitive), or `None`.
    fn log_stats(&self, py: Python<'_>, mnemonic: &str) -> PyResult<Option<Stats>> {
        self.resolve(py, |s| Ok(s.log(mnemonic).map(|lv| Stats::new(lv.stats()))))
    }

    /// A per-sample `LogView` of curve `mnemonic` on **this bore**
    /// (case-insensitive), or `None` — reach `.values()`/`.md()`/`.at_md()` on a
    /// named sidetrack (weakness W3), not just aggregate stats.
    fn log(&self, py: Python<'_>, mnemonic: &str) -> PyResult<Option<LogView>> {
        let exists = self.resolve(py, |s| Ok(s.log(mnemonic).is_some()))?;
        Ok(exists.then(|| LogView {
            well: self.well.clone_ref(py),
            mnemonic: mnemonic.to_string(),
            top_name: None,
            bore: Some(self.label.clone()),
        }))
    }

    /// Add a calculated log on this bore. Raises unless `overwrite=True` when a
    /// curve with the same mnemonic already exists on this bore.
    #[pyo3(signature = (mnemonic, md, values, unit = "", overwrite = false))]
    fn assign_log(
        &self,
        py: Python<'_>,
        mnemonic: &str,
        md: Vec<f64>,
        values: Vec<f64>,
        unit: &str,
        overwrite: bool,
    ) -> PyResult<()> {
        self.mutate(py, |s| {
            if s.log(mnemonic).is_some() && !overwrite {
                return Err(PyValueError::new_err(format!(
                    "log '{mnemonic}' already exists on this bore"
                )));
            }
            let mut log = RsLog::new(mnemonic, unit, md, values)
                .map_err(|e| PyValueError::new_err(e.to_string()))?;
            log.record_history(format!("sidetrack.assign_log(name={mnemonic})"));
            if overwrite {
                s.retain_logs_except(mnemonic);
            }
            s.add_log(log);
            Ok(())
        })
    }

    /// Fluid-contact picks on this bore as `(name, md)` rows.
    fn contacts(&self, py: Python<'_>) -> PyResult<Vec<(String, f64)>> {
        self.resolve(py, |s| {
            Ok(s.contacts().map(|c| (c.name.clone(), c.md)).collect())
        })
    }

    /// The named fluid-contact pick on this bore as `(name, md)`, or `None`.
    fn contact(&self, py: Python<'_>, name: &str) -> PyResult<Option<(String, f64)>> {
        self.resolve(py, |s| Ok(s.contact(name).map(|c| (c.name.clone(), c.md))))
    }

    /// Net-conditioned per-zone aggregation of curve `value` on this bore: for
    /// each zone, keep only **net** samples (those passing the φ/Sw[/Vsh]
    /// cutoffs, with φ/Sw/Vsh sampled onto `value`'s MDs), then aggregate. With
    /// `geomean=False` (default) returns `[(zone, Stats)]` (net arithmetic — e.g.
    /// NTG-conditioned mean φ/Sw); with `geomean=True` returns
    /// `[(zone, float)]` (net geometric mean — e.g. permeability). Zones with no
    /// net samples are included with an empty `Stats` / `NaN`.
    #[pyo3(signature = (value, phi="PHIE", sw="SW", vsh=None, cut=None, phi_min=None, sw_max=None, vsh_max=None, geomean=false))]
    #[allow(clippy::too_many_arguments)]
    fn net_zone_stats(
        &self,
        py: Python<'_>,
        value: &str,
        phi: &str,
        sw: &str,
        vsh: Option<&str>,
        cut: Option<NetSettings>,
        phi_min: Option<f64>,
        sw_max: Option<f64>,
        vsh_max: Option<f64>,
        geomean: bool,
    ) -> PyResult<Py<PyAny>> {
        use petekio::Cutoffs;
        // A `NetSettings` supplies the base cutoffs (default when absent); the
        // scalar kwargs stay as per-call overrides on top of it.
        let base = cut.map(|c| c.cutoffs()).unwrap_or_default();
        let cut = Cutoffs {
            phi_min: phi_min.unwrap_or(base.phi_min),
            sw_max: sw_max.unwrap_or(base.sw_max),
            vsh_max: vsh_max.unwrap_or(base.vsh_max),
        };
        self.resolve(py, |s| {
            // The net-conditioning crunch is pure Rust — run it off the GIL and
            // marshal the per-zone kept samples to Stats/geomean afterwards.
            let samples = py.detach(|| {
                petekio::analysis::well_tables::net_zone_samples(s, value, phi, sw, vsh, &cut)
            });
            let mut out: Vec<(String, Py<PyAny>)> = Vec::with_capacity(samples.len());
            for (name, kept) in samples {
                let obj: Py<PyAny> = if geomean {
                    petekio::Stats::geomean(&kept)
                        .into_pyobject(py)?
                        .into_any()
                        .unbind()
                } else {
                    Stats::new(petekio::Stats::of(&kept))
                        .into_pyobject(py)?
                        .into_any()
                        .unbind()
                };
                out.push((name, obj));
            }
            Ok(out.into_pyobject(py)?.into_any().unbind())
        })
    }

    /// Each formation zone as `(name, top_md, base_md)`, in MD order.
    fn zones(&self, py: Python<'_>) -> PyResult<Vec<(String, f64, f64)>> {
        self.resolve(py, |s| {
            Ok(s.zones()
                .into_iter()
                .map(|z| (z.name.clone(), z.top_md, z.base_md))
                .collect())
        })
    }

    /// Per-zone stats of curve `mnemonic`. With no `zone`, returns the list of
    /// `(zone_name, Stats)` in lithostratigraphic order. With a `zone` name
    /// (case-insensitive), returns just that zone's `Stats`, or `None` if the
    /// zone has no samples / doesn't exist — a direct
    /// `st.zone_stats("PHIE", "Top A")` instead of `dict(...)["Top A"]`.
    #[pyo3(signature = (mnemonic, zone=None))]
    fn zone_stats(
        &self,
        py: Python<'_>,
        mnemonic: &str,
        zone: Option<&str>,
    ) -> PyResult<Py<PyAny>> {
        self.resolve(py, |s| {
            let all = s.zone_stats(mnemonic);
            match zone {
                Some(z) => match all.into_iter().find(|(n, _)| n.eq_ignore_ascii_case(z)) {
                    Some((_, st)) => Ok(Stats::new(st).into_pyobject(py)?.into_any().unbind()),
                    None => Ok(py.None()),
                },
                None => {
                    let list: Vec<(String, Stats)> =
                        all.into_iter().map(|(n, st)| (n, Stats::new(st))).collect();
                    Ok(list.into_pyobject(py)?.into_any().unbind())
                }
            }
        })
    }

    fn __repr__(&self) -> String {
        format!("Sidetrack(label={:?})", self.label)
    }
}

enum TopValue {
    Md(f64),
    Hit(petekio::SurfaceIntersection, Option<usize>),
}

fn top_value(value: &Bound<'_, PyAny>) -> PyResult<TopValue> {
    if let Ok(md) = value.extract::<f64>() {
        return Ok(TopValue::Md(md));
    }
    if let Ok(hit) = value.extract::<PyRef<'_, crate::intersection::SurfaceIntersection>>() {
        return Ok(TopValue::Hit(hit.inner.clone(), hit.project_token));
    }
    Err(pyo3::exceptions::PyTypeError::new_err(
        "top depth must be a finite MD number or SurfaceIntersection",
    ))
}

fn validate_top_project(value: &TopValue, expected: usize) -> PyResult<()> {
    if let TopValue::Hit(_, Some(actual)) = value {
        if *actual != expected {
            return Err(PyValueError::new_err(
                "intersection belongs to a different project",
            ));
        }
    }
    Ok(())
}

/// The depth interval a top names: a view resolving its `Top` on the well.
#[pyclass(name = "Interval")]
pub struct Interval {
    well: Py<Well>,
    top_name: String,
}

impl Interval {
    /// Resolve the borrowed Rust interval and run `f` over it.
    fn resolve<R>(
        &self,
        py: Python<'_>,
        f: impl FnOnce(&petekio::Interval<'_>) -> PyResult<R>,
    ) -> PyResult<R> {
        let w = self.well.borrow(py);
        w.with_well(py, |rw| {
            let iv = rw.top(&self.top_name).ok_or_else(|| {
                PyValueError::new_err(format!("top '{}' no longer present", self.top_name))
            })?;
            f(&iv)
        })
    }
}

#[pymethods]
impl Interval {
    #[getter]
    fn name(&self, py: Python<'_>) -> PyResult<String> {
        self.resolve(py, |iv| Ok(iv.name.clone()))
    }

    #[getter]
    fn top_md(&self, py: Python<'_>) -> PyResult<f64> {
        self.resolve(py, |iv| Ok(iv.top_md))
    }

    #[getter]
    fn base_md(&self, py: Python<'_>) -> PyResult<f64> {
        self.resolve(py, |iv| Ok(iv.base_md))
    }

    /// The interval thickness in measured depth (`base_md - top_md`).
    fn thickness_md(&self, py: Python<'_>) -> PyResult<f64> {
        self.resolve(py, |iv| Ok(iv.thickness_md()))
    }

    /// The log `mnemonic` clipped to this interval, or `None`.
    fn log(&self, py: Python<'_>, mnemonic: &str) -> PyResult<Option<LogView>> {
        let present = self.resolve(py, |iv| Ok(iv.log(mnemonic).is_some()))?;
        Ok(present.then(|| LogView {
            well: self.well.clone_ref(py),
            mnemonic: mnemonic.to_string(),
            top_name: Some(self.top_name.clone()),
            bore: None,
        }))
    }

    /// Dynamic log access: `interval.ntg` → the log's `Stats` over this
    /// interval. Falls back to a normal `AttributeError` for unknown names.
    fn __getattr__(&self, py: Python<'_>, name: String) -> PyResult<Stats> {
        if name.starts_with('_') {
            return Err(PyAttributeError::new_err(name));
        }
        let stats = self.resolve(py, |iv| Ok(iv.log(&name).map(|lv| lv.stats())))?;
        match stats {
            Some(s) => Ok(Stats::new(s)),
            None => Err(PyAttributeError::new_err(format!(
                "'Interval' object has no attribute or log '{name}'"
            ))),
        }
    }

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        self.resolve(py, |iv| {
            Ok(format!(
                "Interval(name={:?}, top_md={}, base_md={})",
                iv.name, iv.top_md, iv.base_md
            ))
        })
    }
}

/// A view onto a well log: the full curve, an interval clip, and/or a named
/// bore. When `bore` is set the view resolves on that sidetrack (else the main
/// bore); when `top_name` is set it is clipped to that interval.
#[pyclass(name = "LogView")]
pub struct LogView {
    well: Py<Well>,
    mnemonic: String,
    /// When set, the view is clipped to this top's interval.
    top_name: Option<String>,
    /// When set, the view resolves on this sidetrack rather than the main bore.
    bore: Option<String>,
}

impl LogView {
    fn resolve<R>(
        &self,
        py: Python<'_>,
        f: impl FnOnce(&petekio::LogView<'_>) -> PyResult<R>,
    ) -> PyResult<R> {
        let w = self.well.borrow(py);
        w.with_well(py, |rw| {
            let view = match (&self.bore, &self.top_name) {
                (Some(b), Some(t)) => rw
                    .sidetrack(b)
                    .and_then(|st| st.top(t))
                    .and_then(|iv| iv.log(&self.mnemonic)),
                (Some(b), None) => rw.sidetrack(b).and_then(|st| st.log(&self.mnemonic)),
                (None, Some(t)) => rw.top(t).and_then(|iv| iv.log(&self.mnemonic)),
                (None, None) => rw.log(&self.mnemonic),
            };
            let view = view.ok_or_else(|| {
                PyValueError::new_err(format!("log '{}' no longer present", self.mnemonic))
            })?;
            f(&view)
        })
    }
}

#[pymethods]
impl LogView {
    /// NaN-skipping summary statistics of the view's values.
    fn stats(&self, py: Python<'_>) -> PyResult<Stats> {
        self.resolve(py, |v| Ok(Stats::new(v.stats())))
    }

    /// Geometric mean of the view's positive values — the natural average for
    /// permeability (weakness W4).
    fn geomean(&self, py: Python<'_>) -> PyResult<f64> {
        self.resolve(py, |v| Ok(petekio::Stats::geomean(v.values())))
    }

    /// The view's values as a `list[float]`.
    fn values(&self, py: Python<'_>) -> PyResult<Vec<f64>> {
        self.resolve(py, |v| Ok(v.values().to_vec()))
    }

    /// Human-readable operation history for the source log/view.
    fn history(&self, py: Python<'_>) -> PyResult<Vec<String>> {
        self.resolve(py, |v| Ok(v.history().to_vec()))
    }

    /// The view's measured depths as a `list[float]`.
    fn md(&self, py: Python<'_>) -> PyResult<Vec<f64>> {
        self.resolve(py, |v| Ok(v.md().to_vec()))
    }

    /// Linearly interpolated value at measured depth `md`, or `None`.
    fn at_md(&self, py: Python<'_>, md: f64) -> PyResult<Option<f64>> {
        self.resolve(py, |v| Ok(v.at_md(md)))
    }

    /// Batched [`at_md`](Self::at_md): one interpolated value per depth in
    /// `depths` (`None` outside the view's span). Resolves the
    /// well→bore→top→log chain **once** for the whole slice instead of re-walking
    /// it per call — the loop-shaped fast path (`[lv.at_md(d) for d in depths]`).
    fn at_md_many(&self, py: Python<'_>, depths: Vec<f64>) -> PyResult<Vec<Option<f64>>> {
        self.resolve(py, |v| Ok(depths.iter().map(|&d| v.at_md(d)).collect()))
    }

    /// The view's `(values, md)` as two `list[float]` in a **single** call — one
    /// chain resolution for the common "read both aligned arrays" shape, instead
    /// of a separate `.values()` + `.md()` re-walk.
    fn values_md(&self, py: Python<'_>) -> PyResult<(Vec<f64>, Vec<f64>)> {
        self.resolve(py, |v| Ok((v.values().to_vec(), v.md().to_vec())))
    }

    fn __len__(&self, py: Python<'_>) -> PyResult<usize> {
        self.resolve(py, |v| Ok(v.md().len()))
    }

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        self.resolve(py, |v| {
            Ok(format!(
                "LogView(mnemonic={:?}, n={})",
                self.mnemonic,
                v.md().len()
            ))
        })
    }
}
