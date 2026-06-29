//! Petrel well-tops reader — the multi-well `# Petrel well tops` export: a
//! `BEGIN HEADER … END HEADER` column block, then space-delimited rows with
//! **quoted** Surface/Well names (may contain spaces) and `-999` nulls.
//!
//! `io` sits below `core`, so this returns primitives ([`PetrelTop`]); the
//! manager routes each record to the matching well + bore. Imports only from
//! `foundation`. Column layout is the standard Petrel order: `X Y Z … MD …
//! Type Surface Well` (indices 0,1,2,6,8,9,10).

use crate::foundation::Result;
use std::path::Path;

/// One picked top: which well, which surface, at what measured depth.
#[derive(Debug, Clone)]
pub struct PetrelTop {
    pub well: String,
    pub surface: String,
    pub md: f64,
}

/// The Petrel null sentinel.
const NULL: f64 = -999.0;

/// Parse a Petrel well-tops file. Rows with a null (`-999`) or non-finite MD are
/// skipped (no depth pick). Surface/Well come from the quoted columns.
pub fn load(path: &Path) -> Result<Vec<PetrelTop>> {
    let text = std::fs::read_to_string(path)?;
    let mut in_data = false;
    let mut out = Vec::new();
    for line in text.lines() {
        let t = line.trim();
        if t.eq_ignore_ascii_case("END HEADER") {
            in_data = true;
            continue;
        }
        if !in_data || t.is_empty() || t.starts_with('#') {
            continue;
        }
        let f = tokenize(t);
        // X Y Z TWT TWT age MD PVD Type Surface Well …
        if f.len() < 11 {
            continue;
        }
        let Ok(md) = f[6].parse::<f64>() else {
            continue;
        };
        if !md.is_finite() || (md - NULL).abs() < 1e-6 {
            continue;
        }
        out.push(PetrelTop {
            well: f[10].clone(),
            surface: f[9].clone(),
            md,
        });
    }
    Ok(out)
}

/// Split a row on whitespace, keeping `"…"`-quoted fields (with internal spaces)
/// as single tokens; quotes are stripped from the result.
fn tokenize(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut buf = String::new();
    let mut in_quote = false;
    for ch in line.chars() {
        match ch {
            '"' => in_quote = !in_quote,
            c if c.is_whitespace() && !in_quote => {
                if !buf.is_empty() {
                    out.push(std::mem::take(&mut buf));
                }
            }
            c => buf.push(c),
        }
    }
    if !buf.is_empty() {
        out.push(buf);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn parses_quoted_rows_skips_null_md() {
        let body = "\
# Petrel well tops
# Unit in depth: m
VERSION 2
BEGIN HEADER
X
Y
Z
TWT picked
TWT auto
Geological age
MD
PVD auto
Type
Surface
Well
END HEADER
556070.25 6810852.55 -2506.67 -999 -999 -999 2531.79 -2506.67 Horizon \"Agat top\" \"36/7-3\"
556080.10 6810860.10 -2520.00 -999 -999 -999 -999 -999 Horizon \"No Pick\" \"36/7-5 B\"
556090.00 6810870.00 -2600.00 -999 -999 -999 2620.50 -2600.0 Horizon \"Cerisa Main top\" \"36/7-5 B\"
";
        let p = std::env::temp_dir().join("petekio_petrel_tops.tops");
        std::fs::File::create(&p)
            .unwrap()
            .write_all(body.as_bytes())
            .unwrap();
        let tops = load(&p).unwrap();
        // Two valid picks (the -999 MD row is skipped).
        assert_eq!(tops.len(), 2);
        assert_eq!(tops[0].well, "36/7-3");
        assert_eq!(tops[0].surface, "Agat top");
        assert!((tops[0].md - 2531.79).abs() < 1e-9);
        assert_eq!(tops[1].well, "36/7-5 B"); // quoted name with a space preserved
        assert_eq!(tops[1].surface, "Cerisa Main top");
    }
}
