//! Declarative ingest/interpretation **spec** value-objects — the petek house
//! spec pattern applied to petekio's Python surface.
//!
//! A spec says WHAT: it is immutable-valued, serialises to/from a plain dict (a
//! scenario is a savable file), compares by value, derives via `.replace(...)`,
//! and pretty-prints as its domain table. Each wraps a core `petekio` value type
//! (`Cutoffs` / `NameMap` + `StratHints`) so the Rust bindings extract it
//! natively and the frozen affordances live in one home.
//!
//! - [`NetSettings`] — the φ/Sw/Vsh reservoir cutoffs (wraps `petekio::Cutoffs`);
//!   consumed by `net_zone_stats`, `zone_table`, and `view`.
//! - [`IngestSpec`] — declarative load-time canonicalization: curve aliases +
//!   strat-order hints + declared unit; applied at `load_well`/`load_well_tops`.

use crate::parse_unit;
use petekio::{Cutoffs, NameMap, StratHints, Unit};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyDict;
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// NetSettings — φ/Sw/Vsh cutoffs
// ---------------------------------------------------------------------------

/// Reservoir net cutoffs as a frozen spec: a sample is **net** iff
/// `phi >= phi_min` AND `sw <= sw_max` AND `vsh <= vsh_max`. Wraps the core
/// `petekio::Cutoffs`. Defaults are the generic clastic starting point
/// (φ≥0.08 / Sw≤0.5 / Vsh≤0.5). Accepted by `net_zone_stats(cut=)`,
/// `zone_table(cut=)`, and `view(spec=ViewSpec(cutoff=))`.
#[pyclass(name = "NetSettings", frozen, eq, from_py_object)]
#[derive(Clone, PartialEq)]
pub struct NetSettings {
    inner: Cutoffs,
}

impl NetSettings {
    /// The wrapped core cutoffs (for the binding methods that condition on them).
    pub(crate) fn cutoffs(&self) -> Cutoffs {
        self.inner
    }
}

#[pymethods]
impl NetSettings {
    #[new]
    #[pyo3(signature = (phi_min=0.08, sw_max=0.5, vsh_max=0.5))]
    fn new(phi_min: f64, sw_max: f64, vsh_max: f64) -> Self {
        NetSettings {
            inner: Cutoffs {
                phi_min,
                sw_max,
                vsh_max,
            },
        }
    }

    #[getter]
    fn phi_min(&self) -> f64 {
        self.inner.phi_min
    }
    #[getter]
    fn sw_max(&self) -> f64 {
        self.inner.sw_max
    }
    #[getter]
    fn vsh_max(&self) -> f64 {
        self.inner.vsh_max
    }

    /// A plain, JSON-able dict `{spec, phi_min, sw_max, vsh_max}` — a scenario is
    /// a savable file. The `"spec"` type tag names the spec (R7 round-trip rule).
    fn to_dict<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let d = PyDict::new(py);
        d.set_item("spec", "NetSettings")?;
        d.set_item("phi_min", self.inner.phi_min)?;
        d.set_item("sw_max", self.inner.sw_max)?;
        d.set_item("vsh_max", self.inner.vsh_max)?;
        Ok(d)
    }

    /// Rebuild from a `{phi_min, sw_max, vsh_max}` dict (missing keys → defaults).
    #[staticmethod]
    fn from_dict(d: &Bound<'_, PyDict>) -> PyResult<Self> {
        let def = Cutoffs::default();
        let get = |k: &str, dflt: f64| -> PyResult<f64> {
            match d.get_item(k)? {
                Some(v) => v.extract(),
                None => Ok(dflt),
            }
        };
        Ok(NetSettings {
            inner: Cutoffs {
                phi_min: get("phi_min", def.phi_min)?,
                sw_max: get("sw_max", def.sw_max)?,
                vsh_max: get("vsh_max", def.vsh_max)?,
            },
        })
    }

    /// A derived spec with the named fields overridden (a scenario knob):
    /// `high = base.replace(phi_min=0.10)`.
    #[pyo3(signature = (phi_min=None, sw_max=None, vsh_max=None))]
    fn replace(&self, phi_min: Option<f64>, sw_max: Option<f64>, vsh_max: Option<f64>) -> Self {
        NetSettings {
            inner: Cutoffs {
                phi_min: phi_min.unwrap_or(self.inner.phi_min),
                sw_max: sw_max.unwrap_or(self.inner.sw_max),
                vsh_max: vsh_max.unwrap_or(self.inner.vsh_max),
            },
        }
    }

    /// The domain cutoff table.
    fn __repr__(&self) -> String {
        format!(
            "NetSettings\n  phi_min  >=  {:.3}\n  sw_max   <=  {:.3}\n  vsh_max  <=  {:.3}",
            self.inner.phi_min, self.inner.sw_max, self.inner.vsh_max
        )
    }
}

// ---------------------------------------------------------------------------
// IngestSpec — declarative load-time canonicalization
// ---------------------------------------------------------------------------

/// Declarative load-time canonicalization: `IngestSpec(aliases={raw: canonical},
/// strat_hints=[("A","B"), ...], unit="m")`. Applied at `load_well(ingest=)` /
/// `load_well_tops(ingest=)` — it replaces the order-dependent sticky
/// `aliases=`/`strat_hint(...)` mutation with a value you pass explicitly per
/// load. `aliases` canonicalizes curve mnemonics; `strat_hints` seed the
/// stratigraphic column order (each `("above","below")` or `"A < B"` shorthand,
/// resolved at apply); `unit` (optional) declares the project length unit the
/// load expects (a loud guard, checked at apply against the project unit).
#[pyclass(name = "IngestSpec", frozen, eq, from_py_object)]
#[derive(Clone, PartialEq)]
pub struct IngestSpec {
    aliases: NameMap,
    strat_hints: StratHints,
    unit: Option<Unit>,
    unit_label: Option<String>,
}

impl IngestSpec {
    pub(crate) fn aliases_value(&self) -> &NameMap {
        &self.aliases
    }
    pub(crate) fn strat_hints_value(&self) -> &StratHints {
        &self.strat_hints
    }
    pub(crate) fn unit_value(&self) -> Option<Unit> {
        self.unit
    }
}

/// Parse a Python `strat_hints` sequence — each item a `(above, below)` tuple or
/// an `"A < B"` / `"A > B"` shorthand string — into a core `StratHints`.
fn parse_strat_hints(hints: Option<&Bound<'_, PyAny>>) -> PyResult<StratHints> {
    let mut out = StratHints::new();
    let Some(seq) = hints else {
        return Ok(out);
    };
    for item in seq.try_iter()? {
        let item = item?;
        if let Ok((a, b)) = item.extract::<(String, String)>() {
            // A (above, below) tuple.
            out.push(a, b);
        } else if let Ok(spec) = item.extract::<String>() {
            // An "A < B" / "A > B" shorthand string.
            out.push_spec(&spec)
                .map_err(|e| PyValueError::new_err(e.to_string()))?;
        } else if let Ok(pair) = item.extract::<Vec<String>>() {
            // A 2-element list (a tuple survives a JSON round-trip as a list).
            if pair.len() != 2 {
                return Err(PyValueError::new_err(
                    "IngestSpec.strat_hints: a list hint must have exactly 2 elements \
                     (above, below)",
                ));
            }
            out.push(pair[0].clone(), pair[1].clone());
        } else {
            return Err(PyValueError::new_err(
                "IngestSpec.strat_hints: each hint must be a (above, below) tuple/list \
                 or an 'A < B' / 'A > B' string",
            ));
        }
    }
    Ok(out)
}

#[pymethods]
impl IngestSpec {
    #[new]
    #[pyo3(signature = (aliases=None, strat_hints=None, unit=None))]
    fn new(
        aliases: Option<HashMap<String, String>>,
        strat_hints: Option<&Bound<'_, PyAny>>,
        unit: Option<String>,
    ) -> PyResult<Self> {
        let aliases = aliases.map(NameMap::from_pairs).unwrap_or_default();
        let strat_hints = parse_strat_hints(strat_hints)?;
        let (unit, unit_label) = match unit {
            Some(s) => (Some(parse_unit(&s)?), Some(s)),
            None => (None, None),
        };
        Ok(IngestSpec {
            aliases,
            strat_hints,
            unit,
            unit_label,
        })
    }

    /// The `{alias: canonical}` map (aliases are stored lowercased).
    #[getter]
    fn aliases<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let d = PyDict::new(py);
        for (a, c) in self.aliases.pairs() {
            d.set_item(a, c)?;
        }
        Ok(d)
    }

    /// The strat-order hints as `[(above, below), ...]`.
    #[getter]
    fn strat_hints(&self) -> Vec<(String, String)> {
        self.strat_hints.pairs().to_vec()
    }

    /// The declared length unit label (`"m"`/`"ft"`), or `None`.
    #[getter]
    fn unit(&self) -> Option<String> {
        self.unit_label.clone()
    }

    /// A plain, JSON-able dict `{spec, aliases, strat_hints, unit}` — round-trips
    /// via `from_dict`. The `"spec"` type tag names the spec (R7 round-trip rule).
    fn to_dict<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let d = PyDict::new(py);
        d.set_item("spec", "IngestSpec")?;
        let aliases = PyDict::new(py);
        for (a, c) in self.aliases.pairs() {
            aliases.set_item(a, c)?;
        }
        d.set_item("aliases", aliases)?;
        d.set_item("strat_hints", self.strat_hints.pairs().to_vec())?;
        d.set_item("unit", self.unit_label.clone())?;
        Ok(d)
    }

    /// Rebuild from a `{aliases, strat_hints, unit}` dict (as `to_dict` emits).
    #[staticmethod]
    fn from_dict(d: &Bound<'_, PyDict>) -> PyResult<Self> {
        let aliases: Option<HashMap<String, String>> = match d.get_item("aliases")? {
            Some(v) if !v.is_none() => Some(v.extract()?),
            _ => None,
        };
        let strat_hints = d.get_item("strat_hints")?;
        let unit: Option<String> = match d.get_item("unit")? {
            Some(v) if !v.is_none() => Some(v.extract()?),
            _ => None,
        };
        IngestSpec::new(aliases, strat_hints.as_ref(), unit)
    }

    /// A derived spec with the named fields overridden. Any omitted field keeps
    /// this spec's value; a supplied field fully replaces it.
    #[pyo3(signature = (aliases=None, strat_hints=None, unit=None))]
    fn replace(
        &self,
        aliases: Option<HashMap<String, String>>,
        strat_hints: Option<&Bound<'_, PyAny>>,
        unit: Option<String>,
    ) -> PyResult<Self> {
        let aliases = match aliases {
            Some(m) => NameMap::from_pairs(m),
            None => self.aliases.clone(),
        };
        let strat_hints = match strat_hints {
            Some(_) => parse_strat_hints(strat_hints)?,
            None => self.strat_hints.clone(),
        };
        let (unit, unit_label) = match unit {
            Some(s) => (Some(parse_unit(&s)?), Some(s)),
            None => (self.unit, self.unit_label.clone()),
        };
        Ok(IngestSpec {
            aliases,
            strat_hints,
            unit,
            unit_label,
        })
    }

    /// The domain ingest table.
    fn __repr__(&self) -> String {
        let unit = self
            .unit_label
            .clone()
            .unwrap_or_else(|| "(project)".into());
        format!(
            "IngestSpec\n  aliases:  {}\n  hints:    {}\n  unit:     {}",
            self.aliases, self.strat_hints, unit
        )
    }
}

/// Register the spec pyclasses on the module.
pub(crate) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<NetSettings>()?;
    m.add_class::<IngestSpec>()?;
    Ok(())
}
