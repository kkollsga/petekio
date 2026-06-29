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

mod geodata;
mod geometry;
mod points;
mod stats;
mod surface;
mod trajectory;
mod well;

use petekio::{GeoError, GridMethod};
use pyo3::exceptions::{PyTypeError, PyValueError};
use pyo3::prelude::*;

/// Convert a Rust `GeoError` into a Python `ValueError`.
pub(crate) fn to_pyerr(e: GeoError) -> PyErr {
    PyValueError::new_err(e.to_string())
}

/// Parse a project length unit from a string (`"ft"`/`"feet"` or
/// `"m"`/`"metre(s)"`/`"meter(s)"`, case-insensitive).
pub(crate) fn parse_unit(s: &str) -> PyResult<petekio::Unit> {
    match s.trim().to_ascii_lowercase().as_str() {
        "ft" | "feet" | "foot" => Ok(petekio::Unit::Feet),
        "m" | "metre" | "metres" | "meter" | "meters" => Ok(petekio::Unit::Metres),
        other => Err(PyValueError::new_err(format!(
            "unknown unit '{other}' (expected 'ft' or 'm')"
        ))),
    }
}

/// Parse a gridding method name (`"nearest"`, `"idw"`/`"inverse_distance"`, or
/// `"min_curvature"`/`"minimum_curvature"`, case-insensitive).
pub(crate) fn parse_grid_method(s: &str) -> PyResult<GridMethod> {
    match s
        .trim()
        .to_ascii_lowercase()
        .replace([' ', '-'], "_")
        .as_str()
    {
        "nearest" => Ok(GridMethod::Nearest),
        "idw" | "inverse_distance" | "inversedistance" => Ok(GridMethod::InverseDistance),
        "min_curvature" | "minimum_curvature" | "mincurvature" | "minimumcurvature" => {
            Ok(GridMethod::MinimumCurvature)
        }
        other => Err(PyTypeError::new_err(format!(
            "unknown grid method '{other}' (expected 'nearest', 'idw', or 'min_curvature')"
        ))),
    }
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
    m.add_class::<points::PointSet>()?;
    m.add_class::<points::PolygonSet>()?;
    m.add_class::<trajectory::Trajectory>()?;
    m.add_class::<well::Well>()?;
    m.add_class::<well::Interval>()?;
    m.add_class::<well::LogView>()?;
    m.add_class::<geodata::GeoData>()?;
    m.add_class::<geodata::WellsView>()?;
    Ok(())
}
