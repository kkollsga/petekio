//! Scattered-XYZ CSV reader — wraps the `csv` crate behind petekIO types.
//!
//! Reads a headered CSV, taking X/Y/Z from the named columns. Every *other*
//! column becomes a `f64` attribute column **iff** all its cells parse as
//! numbers; a column with any non-numeric cell is silently dropped (it is text
//! metadata, not data). Rows with a non-numeric X/Y/Z are an error (readers
//! validate on load). Imports only from `foundation` plus the shared
//! imported-point payload.

use crate::foundation::{GeoError, Result};
use crate::io::PointData;
use indexmap::IndexMap;
use std::path::Path;

/// Read XYZ + attribute columns into the standard imported-point payload.
pub fn load(path: &Path, x: &str, y: &str, z: &str) -> Result<PointData> {
    let mut rdr =
        csv::Reader::from_path(path).map_err(|e| crate::io::csv_error("points CSV", e))?;
    let headers: Vec<String> = rdr
        .headers()
        .map_err(|e| crate::io::csv_error("points CSV: bad header", e))?
        .iter()
        .map(str::to_string)
        .collect();
    let find = |col: &str| {
        headers
            .iter()
            .position(|h| h == col)
            .ok_or_else(|| GeoError::NotFound(format!("points CSV: column '{col}'")))
    };
    let (xi, yi, zi) = (find(x)?, find(y)?, find(z)?);

    // Candidate attribute columns: every column that is not X/Y/Z.
    let attr_cols: Vec<usize> = (0..headers.len())
        .filter(|i| *i != xi && *i != yi && *i != zi)
        .collect();

    let mut coords: Vec<[f64; 3]> = Vec::new();
    // Parsed cells per attribute column; `None` once a non-numeric cell appears.
    let mut raw: Vec<Option<Vec<f64>>> = vec![Some(Vec::new()); attr_cols.len()];

    let parse = |rec: &csv::StringRecord, idx: usize, what: &str, row: usize| -> Result<f64> {
        let cell = rec
            .get(idx)
            .ok_or_else(|| GeoError::Parse(format!("points CSV: row {row}: missing {what}")))?;
        cell.trim().parse::<f64>().map_err(|e| {
            GeoError::Parse(format!("points CSV: row {row}: bad {what} '{cell}': {e}"))
        })
    };

    for (row, rec) in rdr.records().enumerate() {
        let rec = rec.map_err(|e| crate::io::csv_error(&format!("points CSV: row {row}"), e))?;
        let xv = parse(&rec, xi, x, row)?;
        let yv = parse(&rec, yi, y, row)?;
        let zv = parse(&rec, zi, z, row)?;
        coords.push([xv, yv, zv]);
        for (slot, &col) in raw.iter_mut().zip(&attr_cols) {
            let Some(buf) = slot else { continue };
            match rec.get(col).map(str::trim).map(str::parse::<f64>) {
                Some(Ok(v)) => buf.push(v),
                _ => *slot = None, // non-numeric (or missing) → drop this column
            }
        }
    }

    let mut attrs = IndexMap::new();
    for (slot, &col) in raw.into_iter().zip(&attr_cols) {
        if let Some(buf) = slot {
            attrs.insert(headers[col].clone(), buf);
        }
    }
    PointData::new(coords, attrs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error;

    #[test]
    fn missing_file_is_io_error_with_source() {
        let p = std::env::temp_dir().join("petekio_no_such_points_9x8y7.csv");
        let _ = std::fs::remove_file(&p);
        let err = load(&p, "x", "y", "z").unwrap_err();
        // Routed as I/O (not stringified into Parse) so `source()` reaches the OS
        // error — a `NotFound` io::Error is recoverable at the call site.
        match &err {
            GeoError::Io(io) => assert_eq!(io.kind(), std::io::ErrorKind::NotFound),
            other => panic!("expected GeoError::Io, got {other:?}"),
        }
        assert!(
            err.source().is_some(),
            "Io error must expose a source() chain"
        );
    }
}
