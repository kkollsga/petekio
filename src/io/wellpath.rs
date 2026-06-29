//! Petrel `.wellpath` reader — a fully *positioned* well trace (MD / X / Y /
//! TVD / inclination / azimuth) with a `#`-comment header carrying the wellhead
//! XY, the KB datum, and the CRS label.
//!
//! `io` sits below `core`, so this returns primitives ([`WellPath`] /
//! [`WellPathRow`]); the manager turns rows into `Station`/`Point3` and a
//! positioned trajectory (subsea `z = TVD − kb`). Imports only from `foundation`.

use crate::foundation::{GeoError, Result};
use std::path::Path;

/// A parsed `.wellpath`: header datum + the raw survey rows (in file order).
#[derive(Debug, Clone)]
pub struct WellPath {
    /// Wellhead `(x, y)` in the file's CRS (metres).
    pub head: (f64, f64),
    /// Kelly-bushing elevation above MSL (metres); MD/TVD are referenced here.
    pub kb: f64,
    /// CRS label from the header, if found (recorded, never reprojected).
    pub crs: Option<String>,
    pub rows: Vec<WellPathRow>,
}

/// One survey station: measured depth, world XY, TVD (positive-down from KB),
/// inclination, and grid-north azimuth.
#[derive(Debug, Clone, Copy)]
pub struct WellPathRow {
    pub md: f64,
    pub x: f64,
    pub y: f64,
    pub tvd: f64,
    pub inc_deg: f64,
    pub azi_deg: f64,
}

/// Parse a `.wellpath` file.
pub fn load(path: &Path) -> Result<WellPath> {
    let text = std::fs::read_to_string(path)?;
    let (mut hx, mut hy, mut kb): (Option<f64>, Option<f64>, Option<f64>) = (None, None, None);
    let mut crs = None;
    let mut rows = Vec::new();

    for line in text.lines() {
        let t = line.trim();
        if t.is_empty() || t.starts_with('=') {
            continue;
        }
        if let Some(comment) = t.strip_prefix('#') {
            let c = comment.trim();
            let up = c.to_ascii_uppercase();
            if up.contains("WELL HEAD X") {
                hx = header_value(c);
            } else if up.contains("WELL HEAD Y") {
                hy = header_value(c);
            } else if up.contains("DATUM") && up.contains("KB") {
                kb = header_value(c);
            } else if crs.is_none() && (up.contains("UTM") || up.contains("CRS")) {
                crs = Some(c.to_string());
            }
            continue;
        }
        // A data row: ≥11 whitespace fields, all numeric.
        let fields: Vec<&str> = t.split_whitespace().collect();
        if fields.len() < 11 {
            continue; // not a survey row (header text, etc.)
        }
        let nums: Option<Vec<f64>> = fields.iter().map(|f| f.parse::<f64>().ok()).collect();
        let Some(n) = nums else { continue };
        // Cols (1-indexed): 1 MD, 2 X, 3 Y, 4 Z, 5 TVD, 8 AZIM_TN, 9 INCL, 11 AZIM_GN.
        rows.push(WellPathRow {
            md: n[0],
            x: n[1],
            y: n[2],
            tvd: n[4],
            inc_deg: n[8],
            azi_deg: n[10],
        });
    }

    match (hx, hy, kb) {
        (Some(x), Some(y), Some(kb)) if !rows.is_empty() => Ok(WellPath {
            head: (x, y),
            kb,
            crs,
            rows,
        }),
        _ => Err(GeoError::Parse(format!(
            "wellpath '{}': missing wellhead X/Y / KB header or no survey rows",
            path.display()
        ))),
    }
}

/// Extract the trailing number from a header line like
/// `WELL HEAD X-COORDINATE: 558650.0 (m)` (last token that parses as `f64`).
fn header_value(comment: &str) -> Option<f64> {
    let after = comment.rsplit(':').next().unwrap_or(comment);
    after.split_whitespace().find_map(|tok| {
        tok.trim_matches(|c: char| !c.is_ascii_digit() && c != '.' && c != '-')
            .parse::<f64>()
            .ok()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write(tmp: &str, body: &str) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(tmp);
        std::fs::File::create(&p)
            .unwrap()
            .write_all(body.as_bytes())
            .unwrap();
        p
    }

    #[test]
    fn parses_header_and_rows() {
        let body = "\
# WELL TRACE FROM PETREL
# WELL HEAD X-COORDINATE: 558650.0 (m)
# WELL HEAD Y-COORDINATE: 6812460.0 (m)
# WELL DATUM (KB, Kelly bushing, from MSL): 27.3 (m)
# CRS: ED50 / UTM zone 31N
# MD AND TVD ARE REFERENCED AT WELL DATUM
==========
MD  X  Y  Z  TVD  DX  DY  AZIM_TN  INCL  DLS  AZIM_GN
0    558650 6812460   0    0   0 0 145 0  0 145
1200 558650 6812460 -1200 1200 0 0 145 0  0 145
";
        let wp = load(&write("petekio_wp_test.wellpath", body)).unwrap();
        assert_eq!(wp.head, (558650.0, 6812460.0));
        assert_eq!(wp.kb, 27.3);
        assert!(wp.crs.as_deref().unwrap().contains("UTM"));
        assert_eq!(wp.rows.len(), 2);
        assert_eq!(wp.rows[1].md, 1200.0);
        assert_eq!(wp.rows[1].tvd, 1200.0);
        assert_eq!(wp.rows[1].inc_deg, 0.0);
    }

    #[test]
    fn missing_header_errors() {
        let p = write("petekio_wp_bad.wellpath", "MD X Y Z TVD\n0 0 0 0 0\n");
        assert!(load(&p).is_err());
    }
}
