//! Tops CSV reader — wraps the `csv` crate behind petekIO types.
//!
//! Reads a headered CSV of formation tops, picking the name and MD columns by
//! header name. Imports only from `foundation`; `core::Top` builds the domain
//! object. Rows with an unparseable MD are an error (readers validate on load).

use crate::foundation::{GeoError, Result};
use std::path::Path;

/// One tops row: a marker name and its measured depth.
#[derive(Debug, Clone)]
pub struct TopRecord {
    pub name: String,
    pub md: f64,
}

/// Read a headered tops CSV, taking the marker name from column `name_col` and
/// the measured depth from column `md_col` (both matched by header name).
pub fn load(path: &Path, name_col: &str, md_col: &str) -> Result<Vec<TopRecord>> {
    let mut rdr =
        csv::Reader::from_path(path).map_err(|e| GeoError::Parse(format!("tops CSV: {e}")))?;
    let headers = rdr
        .headers()
        .map_err(|e| GeoError::Parse(format!("tops CSV: bad header: {e}")))?;
    let find = |col: &str| {
        headers
            .iter()
            .position(|h| h == col)
            .ok_or_else(|| GeoError::NotFound(format!("tops CSV: column '{col}'")))
    };
    let (ni, mi) = (find(name_col)?, find(md_col)?);

    let mut out = Vec::new();
    for (row, rec) in rdr.records().enumerate() {
        let rec = rec.map_err(|e| GeoError::Parse(format!("tops CSV: row {row}: {e}")))?;
        let name = rec
            .get(ni)
            .ok_or_else(|| GeoError::Parse(format!("tops CSV: row {row}: missing name")))?
            .to_string();
        let md_s = rec
            .get(mi)
            .ok_or_else(|| GeoError::Parse(format!("tops CSV: row {row}: missing md")))?;
        let md = md_s
            .trim()
            .parse::<f64>()
            .map_err(|e| GeoError::Parse(format!("tops CSV: row {row}: bad md '{md_s}': {e}")))?;
        out.push(TopRecord { name, md });
    }
    Ok(out)
}
