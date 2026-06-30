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
use crate::stats::Stats;
use petekio::Well as RsWell;
use pyo3::exceptions::{PyAttributeError, PyImportError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyDict;

/// The `Stats` attribute names a `zone_table` `stats=` may request.
pub(crate) const STAT_ATTRS: &[&str] = &[
    "mean", "sum", "count", "min", "max", "std", "p10", "p50", "p90",
];

/// Extra `stats=` options not on `Stats`: `samples` (= sample count) and `gross`
/// (the zone's MD thickness — geometry, not from the curve).
pub(crate) const EXTRA_STATS: &[&str] = &["samples", "gross"];

fn stat_value(s: &petekio::Stats, name: &str) -> f64 {
    match name {
        "mean" => s.mean,
        "sum" => s.sum,
        "count" | "samples" => s.count as f64,
        "min" => s.min,
        "max" => s.max,
        "std" => s.std,
        "p10" => s.p10,
        "p50" => s.p50,
        "p90" => s.p90,
        _ => f64::NAN,
    }
}

/// Per-sample depth weights for thickness-weighting: each sample carries the MD
/// span it represents (midpoint rule — half-gap to each neighbour, full step at
/// the ends). Uniform sampling → all weights equal → identical to the plain
/// mean, so it only changes irregular / mixed-rate logs.
fn dz_weights(md: &[f64]) -> Vec<f64> {
    let n = md.len();
    match n {
        0 => Vec::new(),
        1 => vec![1.0],
        _ => (0..n)
            .map(|i| {
                if i == 0 {
                    md[1] - md[0]
                } else if i == n - 1 {
                    md[n - 1] - md[n - 2]
                } else {
                    (md[i + 1] - md[i - 1]) / 2.0
                }
            })
            .collect(),
    }
}

/// Build a **tidy** per-`zone × bore` table for `curve` over `bores` (each a
/// display label + its sidetrack) and return it as a `pandas.DataFrame`: columns
/// `zone`, `bore`, then one per requested stat. Bores without a trajectory are
/// skipped. Zones come from each bore's `zones()` (already in lithostratigraphic
/// order), so `zone` is set as an **ordered Categorical** in that order — it
/// survives `pivot`/`groupby`. `count == 0` (zero-thickness / no samples)
/// zone×bore cells are dropped unless `include_empty`. pandas is imported lazily.
///
/// `pivot` → wide (`zone` index × `bore` columns). `aggregate` → grouped by zone
/// with a pooled "all" row first. `pivot` and `aggregate` are mutually exclusive.
/// `weighted` (default true) thickness-weights every average (per-bore and
/// aggregate) by each sample's MD span, so a finely-sampled log doesn't outweigh
/// a coarse one; `false` falls back to the plain sample mean. `decimals` rounds.
/// `stats` may also be `samples` (sample count) or `gross` (zone MD thickness).
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
) -> PyResult<Py<PyAny>> {
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
    let zone_stats = |vals: &[f64], w: &[f64]| {
        if weighted {
            petekio::Stats::weighted(vals, w)
        } else {
            petekio::Stats::of(vals)
        }
    };
    // Optional zone filter: keep only these names (case-insensitive, exact).
    let keep: Option<std::collections::HashSet<String>> =
        zones.map(|z| z.iter().map(|s| s.to_ascii_lowercase()).collect());

    // One pass: per-bore rows (bore-outer, non-empty unless include_empty), the
    // zone first-appearance order, and the pooled (value, weight) pairs per zone.
    let mut order: Vec<String> = Vec::new();
    let mut rows: Vec<(String, String, Vec<f64>)> = Vec::new(); // (zone, bore, stat values)
    let mut pooled: std::collections::HashMap<String, (Vec<f64>, Vec<f64>)> =
        std::collections::HashMap::new();
    for (label, st) in bores {
        if st.trajectories().is_empty() {
            continue; // no md_range — nothing positioned
        }
        for iv in st.zones() {
            if let Some(k) = &keep {
                if !k.contains(&iv.name.to_ascii_lowercase()) {
                    continue; // not in the requested zone subset
                }
            }
            if !order.contains(&iv.name) {
                order.push(iv.name.clone());
            }
            let gross = iv.thickness_md();
            let s = iv.log(curve).map(|l| {
                let w = dz_weights(l.md());
                let st = zone_stats(l.values(), &w);
                let e = pooled.entry(iv.name.clone()).or_default();
                e.0.extend_from_slice(l.values());
                e.1.extend_from_slice(&w);
                st
            });
            let count = s.as_ref().map(|x| x.count).unwrap_or(0);
            if count == 0 && !include_empty {
                continue;
            }
            let vals = stats
                .iter()
                .map(|n| match n.as_str() {
                    "gross" => gross,
                    _ => s.as_ref().map(|s| stat_value(s, n)).unwrap_or(f64::NAN),
                })
                .collect();
            rows.push((iv.name.clone(), label.clone(), vals));
        }
    }
    let pd = py
        .import("pandas")
        .map_err(|_| PyImportError::new_err("zone_table requires pandas — `pip install pandas`"))?;

    // aggregate=True → grouped: per zone a pooled "all" row first (sample-weighted
    // across bores, computed on the re-pooled raw samples so std/percentiles are
    // exact), then the per-bore rows; indexed by (zone, bore).
    if aggregate {
        let mut zone_col: Vec<String> = Vec::new();
        let mut bore_col: Vec<String> = Vec::new();
        let mut stat_cols: Vec<Vec<f64>> = vec![Vec::new(); stats.len()];
        for zone in &order {
            let (pv, pw) = match pooled.get(zone) {
                Some((v, w)) => (v.as_slice(), w.as_slice()),
                None => (&[][..], &[][..]),
            };
            let ps = zone_stats(pv, pw);
            let zrows: Vec<&(String, String, Vec<f64>)> =
                rows.iter().filter(|(z, _, _)| z == zone).collect();
            if ps.count == 0 && zrows.is_empty() && !include_empty {
                continue;
            }
            zone_col.push(zone.clone());
            bore_col.push("all".to_string());
            for (k, name) in stats.iter().enumerate() {
                // `gross` isn't a sample stat — its pooled value is the mean zone
                // thickness across the bores shown.
                let v = if name.as_str() == "gross" {
                    let g: Vec<f64> = zrows.iter().map(|(_, _, vals)| vals[k]).collect();
                    if g.is_empty() {
                        f64::NAN
                    } else {
                        g.iter().sum::<f64>() / g.len() as f64
                    }
                } else {
                    stat_value(&ps, name)
                };
                stat_cols[k].push(v);
            }
            for (_, b, vals) in zrows {
                zone_col.push(zone.clone());
                bore_col.push(b.clone());
                for (k, v) in vals.iter().enumerate() {
                    stat_cols[k].push(*v);
                }
            }
        }
        let data = PyDict::new(py);
        data.set_item("zone", zone_col)?;
        data.set_item("bore", bore_col)?;
        for (k, name) in stats.iter().enumerate() {
            data.set_item(name.as_str(), stat_cols[k].clone())?;
        }
        let mut df = pd.call_method1("DataFrame", (data,))?;
        df = df.call_method1("set_index", (vec!["zone", "bore"],))?;
        if let Some(d) = decimals {
            df = df.call_method1("round", (d,))?;
        }
        return Ok(df.unbind());
    }

    // Flat tidy / pivot. `zone` is an ordered Categorical (built directly as the
    // column, not reassigned — that would trip pandas' copy-on-write warning).
    let mut zone_col: Vec<String> = Vec::with_capacity(rows.len());
    let mut bore_col: Vec<String> = Vec::with_capacity(rows.len());
    let mut stat_cols: Vec<Vec<f64>> = vec![Vec::new(); stats.len()];
    for (z, b, vals) in &rows {
        zone_col.push(z.clone());
        bore_col.push(b.clone());
        for (k, v) in vals.iter().enumerate() {
            stat_cols[k].push(*v);
        }
    }
    let present: std::collections::HashSet<&str> = zone_col.iter().map(String::as_str).collect();
    let cats: Vec<String> = order
        .into_iter()
        .filter(|z| present.contains(z.as_str()))
        .collect();
    let kwargs = PyDict::new(py);
    kwargs.set_item("categories", cats)?;
    kwargs.set_item("ordered", true)?;
    let zone_cat = pd.call_method("Categorical", (zone_col,), Some(&kwargs))?;
    let data = PyDict::new(py);
    data.set_item("zone", zone_cat)?;
    data.set_item("bore", bore_col)?;
    for (k, name) in stats.iter().enumerate() {
        data.set_item(name.as_str(), stat_cols[k].clone())?;
    }
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
    fn xyz(&self, py: Python<'_>, md: f64) -> PyResult<Option<(f64, f64, f64)>> {
        self.with_well(py, |w| Ok(w.xyz(md).map(|p| (p.x, p.y, p.z))))
    }

    /// TVD at measured depth `md`, or `None`.
    fn tvd(&self, py: Python<'_>, md: f64) -> PyResult<Option<f64>> {
        self.with_well(py, |w| Ok(w.tvd(md)))
    }

    /// Measured depth at a given TVD, or `None`.
    fn md_at_tvd(&self, py: Python<'_>, tvd: f64) -> PyResult<Option<f64>> {
        self.with_well(py, |w| Ok(w.md_at_tvd(tvd)))
    }

    /// The interval named by top `name` (case-insensitive), or `None`.
    fn top(slf: Bound<'_, Self>, name: &str) -> PyResult<Option<Interval>> {
        let py = slf.py();
        let exists = slf.borrow().with_well(py, |w| Ok(w.top(name).is_some()))?;
        Ok(exists.then(|| Interval {
            well: slf.clone().unbind(),
            top_name: name.to_string(),
        }))
    }

    /// A full-curve view of log `mnemonic` (case-insensitive), or `None`.
    fn log(slf: Bound<'_, Self>, mnemonic: &str) -> PyResult<Option<LogView>> {
        let py = slf.py();
        let exists = slf
            .borrow()
            .with_well(py, |w| Ok(w.log(mnemonic).is_some()))?;
        Ok(exists.then(|| LogView {
            well: slf.clone().unbind(),
            mnemonic: mnemonic.to_string(),
            top_name: None,
        }))
    }

    /// The coordinate reference system label, or `None`.
    #[getter]
    fn crs(&self, py: Python<'_>) -> PyResult<Option<String>> {
        self.with_well(py, |w| Ok(w.crs().map(|s| s.to_string())))
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
    /// `samples`/`gross`. Requires pandas.
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
            )
        })
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

/// A view onto a well log: either the full curve or an interval clip.
#[pyclass(name = "LogView")]
pub struct LogView {
    well: Py<Well>,
    mnemonic: String,
    /// When set, the view is clipped to this top's interval.
    top_name: Option<String>,
}

impl LogView {
    fn resolve<R>(
        &self,
        py: Python<'_>,
        f: impl FnOnce(&petekio::LogView<'_>) -> PyResult<R>,
    ) -> PyResult<R> {
        let w = self.well.borrow(py);
        w.with_well(py, |rw| {
            let view = match &self.top_name {
                Some(t) => rw.top(t).and_then(|iv| iv.log(&self.mnemonic)),
                None => rw.log(&self.mnemonic),
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

    /// The view's values as a `list[float]`.
    fn values(&self, py: Python<'_>) -> PyResult<Vec<f64>> {
        self.resolve(py, |v| Ok(v.values().to_vec()))
    }

    /// The view's measured depths as a `list[float]`.
    fn md(&self, py: Python<'_>) -> PyResult<Vec<f64>> {
        self.resolve(py, |v| Ok(v.md().to_vec()))
    }

    /// Linearly interpolated value at measured depth `md`, or `None`.
    fn at_md(&self, py: Python<'_>, md: f64) -> PyResult<Option<f64>> {
        self.resolve(py, |v| Ok(v.at_md(md)))
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
