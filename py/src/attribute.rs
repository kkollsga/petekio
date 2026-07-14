//! Python mapping conversion for canonical surface attribute metadata.

use crate::to_pyerr;
use indexmap::IndexMap;
use petekio::{AttributeKind, AttributeMetadata, CodeRecord};
use pyo3::exceptions::PyTypeError;
use pyo3::prelude::*;
use pyo3::types::PyDict;

pub(crate) fn metadata_from_dict(
    name: &str,
    value: &Bound<'_, PyDict>,
) -> PyResult<AttributeMetadata> {
    reject_unknown_keys(
        value,
        &["id", "label", "kind", "units", "codes"],
        "attribute metadata",
    )?;
    let id = optional_string(value, "id")?.unwrap_or_else(|| name.to_string());
    let label = optional_string(value, "label")?.unwrap_or_else(|| id.clone());
    let kind = match optional_string(value, "kind")?
        .unwrap_or_else(|| "continuous".into())
        .as_str()
    {
        "continuous" => AttributeKind::Continuous,
        "categorical" => AttributeKind::Categorical,
        other => {
            return Err(PyTypeError::new_err(format!(
                "attribute metadata kind must be 'continuous' or 'categorical', got '{other}'"
            )))
        }
    };
    let units = optional_string(value, "units")?;
    let codes = match value.get_item("codes")? {
        None => None,
        Some(raw) if raw.is_none() => None,
        Some(raw) => {
            let raw = raw.cast::<PyDict>().map_err(|_| {
                PyTypeError::new_err("attribute metadata codes must be a dict or None")
            })?;
            let mut out = IndexMap::new();
            for (code, record) in raw.iter() {
                let code: String = code.extract().map_err(|_| {
                    PyTypeError::new_err("attribute metadata code keys must be strings")
                })?;
                let record = record.cast::<PyDict>().map_err(|_| {
                    PyTypeError::new_err("attribute metadata code records must be dicts")
                })?;
                reject_unknown_keys(record, &["label", "color"], "attribute code record")?;
                out.insert(
                    code,
                    CodeRecord::new(
                        optional_string(record, "label")?,
                        optional_string(record, "color")?,
                    )
                    .map_err(to_pyerr)?,
                );
            }
            Some(out)
        }
    };
    AttributeMetadata::new(id, label, kind, units, codes).map_err(to_pyerr)
}

pub(crate) fn metadata_to_dict<'py>(
    py: Python<'py>,
    metadata: &AttributeMetadata,
) -> PyResult<Bound<'py, PyDict>> {
    let out = PyDict::new(py);
    out.set_item("id", &metadata.id)?;
    out.set_item("label", &metadata.label)?;
    out.set_item(
        "kind",
        match metadata.kind {
            AttributeKind::Continuous => "continuous",
            AttributeKind::Categorical => "categorical",
        },
    )?;
    out.set_item("units", metadata.units.as_deref())?;
    match &metadata.codes {
        None => out.set_item("codes", py.None())?,
        Some(codes) => {
            let mapped = PyDict::new(py);
            for (code, record) in codes {
                let item = PyDict::new(py);
                item.set_item("label", record.label.as_deref())?;
                item.set_item("color", record.color.as_deref())?;
                mapped.set_item(code, item)?;
            }
            out.set_item("codes", mapped)?;
        }
    }
    Ok(out)
}

fn optional_string(value: &Bound<'_, PyDict>, key: &str) -> PyResult<Option<String>> {
    match value.get_item(key)? {
        None => Ok(None),
        Some(raw) if raw.is_none() => Ok(None),
        Some(raw) => raw.extract::<String>().map(Some).map_err(|_| {
            PyTypeError::new_err(format!("attribute metadata {key} must be a string or None"))
        }),
    }
}

fn reject_unknown_keys(value: &Bound<'_, PyDict>, allowed: &[&str], context: &str) -> PyResult<()> {
    for key in value.keys().iter() {
        let key: String = key
            .extract()
            .map_err(|_| PyTypeError::new_err(format!("{context} keys must be strings")))?;
        if !allowed.contains(&key.as_str()) {
            return Err(PyTypeError::new_err(format!(
                "unknown {context} key '{key}'"
            )));
        }
    }
    Ok(())
}
