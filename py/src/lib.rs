//! Thin PyO3 bindings over the `petekio` Rust library — the source of the
//! `petekio` Python wheel. Bindings only marshal; all logic lives in Rust.
//!
//! This is an early, deliberately small surface (Surface + Stats). It grows to
//! mirror `API.md` as the Rust phases land (wells, logs, points, `GeoData`).

use petekio::{Stats as RsStats, Surface as RsSurface};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

fn to_pyerr(e: petekio::GeoError) -> PyErr {
    PyValueError::new_err(e.to_string())
}

/// Summary statistics over defined (non-NaN) values. Read-only.
#[pyclass(name = "Stats", frozen)]
struct Stats {
    inner: RsStats,
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

    fn __repr__(&self) -> String {
        format!(
            "Stats(count={}, mean={}, min={}, max={}, p50={})",
            self.inner.count, self.inner.mean, self.inner.min, self.inner.max, self.inner.p50
        )
    }
}

/// A regular gridded surface (IRAP/RMS model).
#[pyclass(name = "Surface")]
struct Surface {
    inner: RsSurface,
}

#[pymethods]
impl Surface {
    /// Load an IRAP-classic (ROXAR ASCII) surface from `path`.
    #[staticmethod]
    fn load_irap_classic(path: &str) -> PyResult<Surface> {
        RsSurface::load_irap_classic(path)
            .map(|inner| Surface { inner })
            .map_err(to_pyerr)
    }

    /// Write this surface's primary layer as IRAP-classic ASCII to `path`.
    fn save_irap_classic(&self, path: &str) -> PyResult<()> {
        self.inner.save_irap_classic(path).map_err(to_pyerr)
    }

    /// Bilinear sample at world `(x, y)`; `None` if outside the grid or near an
    /// undefined node.
    fn sample(&self, x: f64, y: f64) -> Option<f64> {
        self.inner.sample(x, y)
    }

    /// Areal extent of nodes whose value is `<= depth`.
    fn area_below(&self, depth: f64) -> f64 {
        self.inner.area_below(depth)
    }

    /// Areal extent of nodes whose value is `>= depth`.
    fn area_above(&self, depth: f64) -> f64 {
        self.inner.area_above(depth)
    }

    /// Summary statistics over the defined nodes.
    fn stats(&self) -> Stats {
        Stats {
            inner: self.inner.stats(),
        }
    }

    #[getter]
    fn ncol(&self) -> usize {
        self.inner.geom.ncol
    }
    #[getter]
    fn nrow(&self) -> usize {
        self.inner.geom.nrow
    }
    #[getter]
    fn rotation_deg(&self) -> f64 {
        self.inner.geom.rotation_deg
    }

    fn __repr__(&self) -> String {
        format!(
            "Surface(ncol={}, nrow={})",
            self.inner.geom.ncol, self.inner.geom.nrow
        )
    }
}

/// The compiled extension module (`petekio._petekio`); re-exported by the
/// `petekio` Python package's `__init__`.
#[pymodule]
fn _petekio(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    m.add_class::<Surface>()?;
    m.add_class::<Stats>()?;
    Ok(())
}
