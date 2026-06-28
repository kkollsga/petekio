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
use pyo3::exceptions::{PyAttributeError, PyValueError};
use pyo3::prelude::*;

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
