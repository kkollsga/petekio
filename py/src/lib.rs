//! Thin PyO3 bindings over the `petekio` Rust library — the source of the
//! `petekio` Python wheel. Bindings only marshal; all logic lives in Rust.
//!
//! The surface mirrors `API.md` §"Python (PyO3) surface": surfaces (operator
//! overloads, attribute access, statistics, volumetrics), wells/logs/tops (the
//! dynamic `w.brent.ntg` `__getattr__` chain), points/polygons, and the
//! `GeoData` project with a broadcastable wells view.
//!
//! Each module wraps the matching Rust type in a `#[pyclass]` and delegates.
//! Numpy/`ndarray` exposure is deliberately out of scope (it would add a numpy
//! dependency); attribute layers are returned as promoted `Surface`s and curve
//! samples as plain `list[float]`.

mod geometry;
mod stats;
mod surface;

use petekio::GeoError;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

/// Convert a Rust `GeoError` into a Python `ValueError`.
pub(crate) fn to_pyerr(e: GeoError) -> PyErr {
    PyValueError::new_err(e.to_string())
}

/// The compiled extension module (`petekio._petekio`); re-exported by the
/// `petekio` Python package's `__init__`.
#[pymodule]
fn _petekio(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    m.add_class::<stats::Stats>()?;
    m.add_class::<geometry::BBox>()?;
    m.add_class::<geometry::GridGeometry>()?;
    m.add_class::<surface::Surface>()?;
    m.add_class::<surface::AttrAccessor>()?;
    Ok(())
}
