//! `Stats` — summary statistics with the fields exposed as read-only attributes
//! (`s.mean`, `s.p50`) plus `percentile`. Mirrors `petekio::Stats`.

use petekio::Stats as RsStats;
use pyo3::prelude::*;

/// Summary statistics over defined (non-NaN) values. Read-only: every field is
/// a getter; arbitrary quantiles come from [`Stats::percentile`].
#[pyclass(name = "Stats", frozen)]
pub struct Stats {
    pub(crate) inner: RsStats,
}

impl Stats {
    /// Wrap a Rust `Stats`.
    pub(crate) fn new(inner: RsStats) -> Stats {
        Stats { inner }
    }
}

#[pymethods]
impl Stats {
    #[getter]
    fn count(&self) -> usize {
        self.inner.count
    }
    #[getter]
    fn mean(&self) -> f64 {
        self.inner.mean
    }
    #[getter]
    fn min(&self) -> f64 {
        self.inner.min
    }
    #[getter]
    fn max(&self) -> f64 {
        self.inner.max
    }
    #[getter]
    fn std(&self) -> f64 {
        self.inner.std
    }
    #[getter]
    fn sum(&self) -> f64 {
        self.inner.sum
    }
    #[getter]
    fn p10(&self) -> f64 {
        self.inner.p10
    }
    #[getter]
    fn p50(&self) -> f64 {
        self.inner.p50
    }
    #[getter]
    fn p90(&self) -> f64 {
        self.inner.p90
    }

    /// Arbitrary percentile, `p` in `[0, 1]`.
    fn percentile(&self, p: f64) -> f64 {
        self.inner.percentile(p)
    }

    /// Geometric mean of positive, non-NaN `values` — the natural average for
    /// log-normal quantities such as permeability. Static: `Stats.geomean([...])`.
    #[staticmethod]
    fn geomean(values: Vec<f64>) -> f64 {
        RsStats::geomean(&values)
    }

    fn __repr__(&self) -> String {
        format!(
            "Stats(count={}, mean={}, min={}, max={}, p50={})",
            self.inner.count, self.inner.mean, self.inner.min, self.inner.max, self.inner.p50
        )
    }
}
