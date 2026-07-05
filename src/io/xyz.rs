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

/// Header signatures of formats that are **not** plain IRAP/RMS `X Y Z` points
/// but would otherwise be silently mis-read — a numeric-looking header block gets
/// parsed as coordinates, yielding a wrong-sized point set with no error
/// (weakness W5: an EarthVision grid through `load_irap_points`). Scans a header
/// window and returns the detected format name if the file is clearly one of
/// them; `None` for a plain `X Y Z` file.
pub(crate) fn foreign_point_format(text: &str) -> Option<&'static str> {
    for line in text.lines().take(60) {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        // LAS section header.
        if t.starts_with('~') {
            return Some("LAS");
        }
        // CPS-3 (grid or lines): `FS*`/`FF*` header records or a `->` block start.
        let first = t
            .split_whitespace()
            .next()
            .unwrap_or("")
            .to_ascii_uppercase();
        if t.starts_with("->")
            || matches!(
                first.as_str(),
                "FSASCI" | "FFASCI" | "FFATTR" | "FSNROW" | "FSXINC" | "FSLIMI"
            )
        {
            return Some("CPS-3");
        }
        // EarthVision grid directive/comment markers.
        let up = t.to_ascii_uppercase();
        if up.contains("EARTHVISION")
            || up.contains("GRID_SIZE")
            || up.contains("GRID_SPACE")
            || (t.starts_with('#') && up.contains("FIELD:"))
        {
            return Some("EarthVision grid");
        }
    }
    None
}

/// Read scattered points: every `X Y Z` line becomes a coordinate. The `999.0`
/// sentinel (if present) is skipped rather than treated as data.
///
/// Format-sniffed on entry: a file whose header is clearly a *different* format
/// (EarthVision grid, CPS-3, LAS) is rejected with a typed [`GeoError::Format`]
/// naming the detected format, rather than mis-parsing its header into wrong
/// coordinates.
pub fn load_points(path: &Path) -> Result<Vec<[f64; 3]>> {
    // Decode permissively (Petrel/EarthVision exports may be Latin-1) so the
    // sniff and parse never choke on a stray non-UTF-8 byte in a name/comment.
    let text = crate::io::decode_latin1(&std::fs::read(path)?);
    if let Some(fmt) = foreign_point_format(&text) {
        return Err(GeoError::Format(format!(
            "IRAP points reader: '{}' looks like {fmt}, not plain X Y Z — use the {fmt} reader",
            path.display()
        )));
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write(name: &str, body: &str) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(name);
        std::fs::File::create(&p)
            .unwrap()
            .write_all(body.as_bytes())
            .unwrap();
        p
    }

    #[test]
    fn plain_xyz_loads() {
        let p = write(
            "petekio_plain_points.xyz",
            "# a comment\n1.0 2.0 -100.0\n3.0 4.0 -110.0\n",
        );
        let pts = load_points(&p).unwrap();
        assert_eq!(pts, vec![[1.0, 2.0, -100.0], [3.0, 4.0, -110.0]]);
    }

    #[test]
    fn earthvision_header_is_rejected() {
        // EarthVision grid ASCII: '#'-directive header, then x y z rows. The
        // header would otherwise be skipped and the body mis-read as a wrong set.
        let p = write(
            "petekio_ev_points.EarthVisionGrid",
            "# Type: scattered data\n# Field: 1 X\n# Grid_size: 2 x 2\n\
             # End:\n1.0 2.0 -100.0\n3.0 4.0 -110.0\n",
        );
        let err = load_points(&p).unwrap_err();
        assert!(matches!(err, GeoError::Format(_)), "got {err:?}");
        assert!(format!("{err}").contains("EarthVision"));
    }

    #[test]
    fn cps3_header_is_rejected() {
        let p = write(
            "petekio_cps3_points.xyz",
            "FSASCI 0 1 2 3\nFSNROW 3 3\n-> 1\n1.0 2.0 -100.0\n",
        );
        let err = load_points(&p).unwrap_err();
        assert!(matches!(err, GeoError::Format(_)), "got {err:?}");
        assert!(format!("{err}").contains("CPS-3"));
    }
}
