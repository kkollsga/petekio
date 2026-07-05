//! `well.view()` support — petekio's producer slice of the well-correlation
//! seam (`petekSuite/dev-docs/designs/well-log-bundle-seam.md`; the viewer's
//! implemented `WellLogBundle` in `petektools/viewer/SCHEMA.md`).
//!
//! Rust gathers the *raw* per-well data — a shared `md`/`tvd` grid, each curve
//! resampled onto it, canonicalized mnemonics + units + core flag, and the
//! well's zones (md → tvd) — into a plain Python dict, then hands it to the pure
//! Python producer `petekio._viewer`, which owns the wire format (base64 f32
//! lane blocks, per-curve range/cutoff/codes, tops ordering, serve/save). Only
//! `petekio._viewer` is imported here; the *optional* `petektools.viewer` runtime
//! dependency is imported lazily inside that module.
//!
//! DAG note: petekio imports NEITHER peteksim NOR petekstatic; the wire schema is
//! duplicated (seam-twin) rather than shared, per the family coupling rule.

use petekio::{Sidetrack, Well as RsWell};
use pyo3::exceptions::{PyImportError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};

/// Resolve the single bore whose logs feed the bundle. An explicit default bore
/// wins; otherwise the one bore carrying curves; a multi-bore well with several
/// log-bearing bores and no default is a loud error (pick one first). A well with
/// no log-bearing bore falls back to the main bore (it may still carry zones).
fn resolve_bore(w: &RsWell) -> PyResult<&Sidetrack> {
    if let Some(label) = w.default_bore() {
        return w.sidetrack(label).ok_or_else(|| {
            PyValueError::new_err(format!("well '{}': default bore '{label}' not found", w.id))
        });
    }
    let with_logs: Vec<&Sidetrack> = w
        .sidetracks()
        .filter(|s| s.logs().next().is_some())
        .collect();
    match with_logs.len() {
        1 => Ok(with_logs[0]),
        0 => Ok(w.main()),
        _ => {
            let labels: Vec<&str> = with_logs.iter().map(|s| s.label.as_str()).collect();
            Err(PyValueError::new_err(format!(
                "well '{}' has {} bores carrying logs ({}) — view() can't pick one; \
                 call set_default_bore(name) first",
                w.id,
                with_logs.len(),
                labels.join(", ")
            )))
        }
    }
}

/// Gather one well's raw log data as a Python dict for `petekio._viewer`.
///
/// Shape: `{id, display_name, x, y, datum_m, md:[f64], tvd:[f64],
/// curves:[{mnemonic, canonical, unit, core, values:[f64]}], zones:[{name,
/// top_md, base_md, top_tvd, base_tvd}]}`. The master `md` grid is the
/// sorted-unique union of the selected curves' depths; every curve is resampled
/// onto it via `LogView::at_md` (linear in-span, `NaN` outside — a curve that
/// doesn't reach a depth simply breaks there). `tvd` is trajectory TVDSS where a
/// trajectory exists, else the documented vertical assumption `md - kb`.
pub(crate) fn raw_log_well(
    py: Python<'_>,
    w: &RsWell,
    filter: Option<&[String]>,
) -> PyResult<Py<PyDict>> {
    let st = resolve_bore(w)?;
    let kb = w.kb;
    // The master-grid resample is pure Rust — gather off the GIL, then marshal.
    let raw = py.detach(|| petekio::analysis::well_tables::gather_raw_logs(st, kb, filter));

    let curves = PyList::empty(py);
    for c in &raw.curves {
        let cd = PyDict::new(py);
        cd.set_item("mnemonic", &c.mnemonic)?;
        cd.set_item("canonical", &c.canonical)?;
        cd.set_item("unit", &c.unit)?;
        cd.set_item("core", c.core)?;
        cd.set_item("values", c.values.clone())?;
        curves.append(cd)?;
    }

    let zones = PyList::empty(py);
    for z in &raw.zones {
        let zd = PyDict::new(py);
        zd.set_item("name", &z.name)?;
        zd.set_item("top_md", z.top_md)?;
        zd.set_item("base_md", z.base_md)?;
        zd.set_item("top_tvd", z.top_tvd)?;
        zd.set_item("base_tvd", z.base_tvd)?;
        zones.append(zd)?;
    }

    let (x, y) = w.head;
    let d = PyDict::new(py);
    d.set_item("id", &w.id)?;
    d.set_item("display_name", &w.id)?;
    d.set_item("x", x)?;
    d.set_item("y", y)?;
    d.set_item("datum_m", kb)?;
    d.set_item("md", raw.md)?;
    d.set_item("tvd", raw.tvd)?;
    d.set_item("curves", curves)?;
    d.set_item("zones", zones)?;
    Ok(d.unbind())
}

/// The loud **spec XOR legacy-kwargs** guard shared by `Well.view` /
/// `WellsView.view`: a `ViewSpec` may not be combined with the legacy WHAT
/// kwargs (curves/tops/flatten_default/phie_cutoff/flags), nor a `ViewSettings`
/// with the legacy HOW kwargs (serve/save).
pub(crate) fn view_xor_guard(
    spec_set: bool,
    settings_set: bool,
    what_set: bool,
    how_set: bool,
) -> PyResult<()> {
    if spec_set && what_set {
        return Err(PyValueError::new_err(
            "view(): pass EITHER spec=ViewSpec(...) OR the legacy curves/tops/\
             flatten_default/phie_cutoff/flags kwargs, not both",
        ));
    }
    if settings_set && how_set {
        return Err(PyValueError::new_err(
            "view(): pass EITHER settings=ViewSettings(...) OR the legacy serve/save \
             kwargs, not both",
        ));
    }
    Ok(())
}

/// Resolve which curves to gather: from `spec.curves` when a `ViewSpec` is
/// given, else the legacy `curves` kwarg.
pub(crate) fn gather_curves(
    py: Python<'_>,
    spec: Option<&Py<PyAny>>,
    curves: Option<Vec<String>>,
) -> PyResult<Option<Vec<String>>> {
    match spec {
        Some(s) => {
            let c = s.bind(py).getattr("curves")?;
            if c.is_none() {
                Ok(None)
            } else {
                Ok(Some(c.extract()?))
            }
        }
        None => Ok(curves),
    }
}

/// Build the `WellLogBundle` from the raw well data via `petekio._viewer.render`
/// and (per the `ViewSettings` / `serve`/`save`) serve or export it. `data` is
/// one raw well dict (`Well.view`) or a list of them (`WellsView.view`). A
/// `ViewSpec`/`ViewSettings` is forwarded through when present; otherwise the
/// legacy per-call kwargs are (only the `Some` ones — `_viewer.render` supplies
/// the defaults). Returns the Python `LogSession`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn render(
    py: Python<'_>,
    data: Bound<'_, PyAny>,
    spec: Option<Py<PyAny>>,
    settings: Option<Py<PyAny>>,
    tops: Option<Py<PyAny>>,
    flatten_default: Option<String>,
    phie_cutoff: Option<f64>,
    flags: Option<Vec<String>>,
    serve: Option<bool>,
    save: Option<String>,
) -> PyResult<Py<PyAny>> {
    let module = py.import("petekio._viewer").map_err(|e| {
        PyImportError::new_err(format!(
            "petekio._viewer (the WellLogBundle producer) failed to import: {e}"
        ))
    })?;
    let kwargs = PyDict::new(py);
    if let Some(s) = spec {
        kwargs.set_item("spec", s)?;
    }
    if let Some(s) = settings {
        kwargs.set_item("settings", s)?;
    }
    if let Some(t) = tops {
        kwargs.set_item("tops", t)?;
    }
    if let Some(f) = flatten_default {
        kwargs.set_item("flatten_default", f)?;
    }
    if let Some(c) = phie_cutoff {
        kwargs.set_item("phie_cutoff", c)?;
    }
    if let Some(fl) = flags {
        kwargs.set_item("flags", fl)?;
    }
    if let Some(sv) = serve {
        kwargs.set_item("serve", sv)?;
    }
    if let Some(s) = save {
        kwargs.set_item("save", s)?;
    }
    Ok(module
        .call_method("render", (data,), Some(&kwargs))?
        .unbind())
}
