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
mod detect;
mod points;
mod shell;
mod specs;
mod stats;
mod structured_surface;
mod tri_surface;
mod surface;
mod trajectory;
mod viewer;
mod well;

use petekio::{GeoError, GridMethod};
use pyo3::exceptions::{PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::wrap_pyfunction;

/// Canonical curve mnemonic for a raw LAS mnemonic (case-insensitive, trimmed,
/// vintage-tag stripped). petekio is the family's curve-name authority — the
/// `WellLogBundle` producer and any consumer normalize through this. An
/// unrecognised mnemonic passes through (vintage-stripped, original case).
#[pyfunction]
fn canonical_mnemonic(raw: &str) -> String {
    petekio::analysis::normalize::canonical_mnemonic(raw)
}

/// Convert a Rust `GeoError` into a Python `ValueError`.
pub(crate) fn to_pyerr(e: GeoError) -> PyErr {
    PyValueError::new_err(e.to_string())
}

/// The display leaf of a project object key: the part after the last `/`
/// (`"Surfaces/IrapClassic_points/Top Agat"` → `"Top Agat"`). This is the
/// dataset name recorded on project-accessor hand-backs (the duck-typed
/// `.name` viewer seam).
pub(crate) fn leaf_name(key: &str) -> String {
    key.rsplit('/').next().unwrap_or(key).trim().to_string()
}

/// Emit a Python `DeprecationWarning` with `msg` (via the `warnings` module, so
/// it respects the interpreter's filters). Used by the legacy sticky-mutation
/// sugar (`load_well(aliases=)`, `strat_hint(...)`) now superseded by
/// `IngestSpec`.
pub(crate) fn deprecation_warning(py: Python<'_>, msg: &str) -> PyResult<()> {
    let warnings = py.import("warnings")?;
    let category = py.get_type::<pyo3::exceptions::PyDeprecationWarning>();
    warnings.call_method1("warn", (msg, category, 2))?;
    Ok(())
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

/// The canonical short label for a project length unit (inverse of
/// [`parse_unit`] for error messages / spec repr).
pub(crate) fn unit_label(u: petekio::Unit) -> &'static str {
    match u {
        petekio::Unit::Feet => "ft",
        petekio::Unit::Metres => "m",
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
    m.add_function(wrap_pyfunction!(canonical_mnemonic, m)?)?;
    m.add_function(wrap_pyfunction!(detect::detect, m)?)?;
    m.add_class::<detect::FormatKind>()?;
    m.add_class::<stats::Stats>()?;
    m.add_class::<geometry::BBox>()?;
    m.add_class::<geometry::GridGeometry>()?;
    m.add_class::<points::TopologyReport>()?;
    m.add_class::<shell::StructuredShell>()?;
    m.add_class::<shell::MeshShell>()?;
    m.add_class::<tri_surface::TriSurface>()?;
    m.add_class::<surface::Surface>()?;
    m.add_class::<surface::AttrAccessor>()?;
    m.add_class::<structured_surface::StructuredMeshSurface>()?;
    m.add_class::<points::PointSet>()?;
    m.add_class::<points::PointColumn>()?;
    m.add_class::<points::PolygonSet>()?;
    m.add_class::<points::PolygonColumn>()?;
    m.add_class::<trajectory::Trajectory>()?;
    m.add_class::<well::Well>()?;
    m.add_class::<well::Sidetrack>()?;
    m.add_class::<well::Interval>()?;
    m.add_class::<well::LogView>()?;
    m.add_class::<geodata::GeoData>()?;
    m.add_class::<geodata::WellsView>()?;
    specs::register(m)?;
    Ok(())
}
