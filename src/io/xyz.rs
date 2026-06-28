//! IRAP/RMS plain XYZ reader — scattered points and polygons.
//!
//! Both formats are whitespace- (or comma-) delimited `X Y Z` lines. For
//! polygons, a line whose coordinate equals the undefined sentinel `999.0`
//! separates one ring from the next (the xtgeo/RMS convention). Blank lines and
//! `#`/`!` comment lines are skipped. Imports only from `foundation`.

use crate::foundation::{GeoError, Result};
use std::path::Path;

/// The polygon ring separator / undefined sentinel in RMS `.pol` files.
const POLY_SEP: f64 = 999.0;

/// Parse one data line into an `(x, y, z)` triple, or `None` for a
/// blank/comment line. Commas are treated as whitespace.
fn parse_xyz(line: &str) -> Result<Option<[f64; 3]>> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') || line.starts_with('!') {
        return Ok(None);
    }
    let mut it = line.split(|c: char| c.is_whitespace() || c == ',');
    let mut next = |what: &str| -> Result<f64> {
        loop {
            let t = it
                .next()
                .ok_or_else(|| GeoError::Parse(format!("XYZ: missing {what} in '{line}'")))?;
            if t.is_empty() {
                continue; // collapse repeated delimiters
            }
            return t
                .parse::<f64>()
                .map_err(|e| GeoError::Parse(format!("XYZ: bad {what} '{t}': {e}")));
        }
    };
    let x = next("x")?;
    let y = next("y")?;
    let z = next("z")?;
    Ok(Some([x, y, z]))
}

/// Read scattered points: every `X Y Z` line becomes a coordinate. The `999.0`
/// sentinel (if present) is skipped rather than treated as data.
pub fn load_points(path: &Path) -> Result<Vec<[f64; 3]>> {
    let text = std::fs::read_to_string(path)?;
    let mut out = Vec::new();
    for line in text.lines() {
        if let Some(p) = parse_xyz(line)? {
            if p[0] == POLY_SEP {
                continue;
            }
            out.push(p);
        }
    }
    Ok(out)
}

/// Read polygons: consecutive `X Y Z` lines form one ring; a `999.0` separator
/// line closes the current ring and starts the next. Empty rings are dropped.
pub fn load_polygons(path: &Path) -> Result<Vec<Vec<[f64; 3]>>> {
    let text = std::fs::read_to_string(path)?;
    let mut rings: Vec<Vec<[f64; 3]>> = Vec::new();
    let mut current: Vec<[f64; 3]> = Vec::new();
    for line in text.lines() {
        let Some(p) = parse_xyz(line)? else { continue };
        if p[0] == POLY_SEP {
            if !current.is_empty() {
                rings.push(std::mem::take(&mut current));
            }
        } else {
            current.push(p);
        }
    }
    if !current.is_empty() {
        rings.push(current);
    }
    Ok(rings)
}
