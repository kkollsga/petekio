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
///
/// Real Petrel/RMS tops exports are **Latin-1/Windows-1252** (Norwegian marker
/// names like `"Blåbær"`), not UTF-8 — so the bytes are decoded through
/// [`decode_latin1`](crate::io::decode_latin1) (as every other reader here does)
/// before parsing, rather than fed to `csv::Reader::from_path`'s strict-UTF-8
/// path where a `0xC5` byte would abort the load.
pub fn load(path: &Path, name_col: &str, md_col: &str) -> Result<Vec<TopRecord>> {
    let bytes = std::fs::read(path)?;
    let text = crate::io::decode_latin1(&bytes);
    let mut rdr = csv::Reader::from_reader(std::io::Cursor::new(text.into_bytes()));
    let headers = rdr
        .headers()
        .map_err(|e| crate::io::csv_error("tops CSV: bad header", e))?;
    let find = |col: &str| {
        headers
            .iter()
            .position(|h| h == col)
            .ok_or_else(|| GeoError::NotFound(format!("tops CSV: column '{col}'")))
    };
    let (ni, mi) = (find(name_col)?, find(md_col)?);

    let mut out = Vec::new();
    for (row, rec) in rdr.records().enumerate() {
        let rec = rec.map_err(|e| crate::io::csv_error(&format!("tops CSV: row {row}"), e))?;
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

#[cfg(test)]
mod latin1_tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn decodes_latin1_marker_names() {
        // 0xE5 = 'å', 0xE6 = 'æ' in Latin-1 (both invalid UTF-8). A real Petrel
        // tops CSV carries them; the reader must decode, not choke.
        let mut body: Vec<u8> = b"name,md\n".to_vec();
        body.extend_from_slice(b"Bl");
        body.push(0xE5); // å
        body.extend_from_slice(b"b");
        body.push(0xE6); // æ
        body.extend_from_slice(b"r,2531.79\n");
        let p = std::env::temp_dir().join("petekio_latin1_tops_csv.csv");
        std::fs::File::create(&p).unwrap().write_all(&body).unwrap();
        let recs = load(&p, "name", "md").unwrap();
        assert_eq!(recs.len(), 1);
        // Proper decode: the bytes became 'å'/'æ', not the '\u{FFFD}' replacement.
        assert_eq!(recs[0].name, "Blåbær");
        assert!((recs[0].md - 2531.79).abs() < 1e-9);
    }
}
