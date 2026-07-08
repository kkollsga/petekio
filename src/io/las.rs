//! LAS (Log ASCII Standard) reader — wraps `las_rs` 0.2 for LAS 1.2/2.0 and
//! carries a **contained internal fallback parser for LAS 3.0** behind the same
//! public API.
//!
//! `las_rs` 0.2 reads only LAS 1.2/2.0; a LAS 3.0 file (`VERS 3.0`, often
//! `DLM COMMA` with `~Log_Definition`/`~Log_Data` sections) previously loaded as
//! a well with **zero curves, silently**. [`load`] now sniffs the `~Version`
//! `VERS`: below 3.0 it defers to `las_rs` (unchanged); at 3.0 it reads the
//! delimited layout with a small self-contained parser here (no third-party 3.0
//! parser), or returns a typed [`GeoError::Format`] naming why (e.g. wrapped
//! data, or no curve section). `las_rs` already maps NULL → `f64::NAN`; the 3.0
//! path does the same. This module imports only from `foundation` (+ the io
//! Latin-1 decoder); `core::Log` builds the domain object from the raw data.

use crate::foundation::{GeoError, Result};
use crate::io::{LogCurveData, LogData};
use std::path::Path;

fn map_err(e: las_rs::LasError) -> GeoError {
    GeoError::Parse(format!("LAS: {e}"))
}

/// Read a LAS file into its md curve plus the remaining curves. LAS 1.2/2.0
/// go through `las_rs`; LAS 3.0 (delimited) through the contained fallback.
pub(crate) fn load(path: &Path) -> Result<LogData> {
    // Petrel LAS exports may be Latin-1 (Norwegian names), not UTF-8 — decode
    // permissively so the version/section sniff can't choke on a stray byte.
    let bytes = std::fs::read(path)?;
    let text = crate::io::decode_latin1(&bytes);
    // Split into `~` sections once and pass them down (both the version sniff and
    // the 3.0 parse need them — re-splitting was redundant).
    let secs = split_sections(&text);
    match detect_version(&secs) {
        Some(v) if v >= 3.0 => parse_las3(&secs),
        _ => load_via_las_rs(path),
    }
}

/// LAS 1.2/2.0 read via `las_rs` (the original path; unchanged behaviour).
fn load_via_las_rs(path: &Path) -> Result<LogData> {
    let las = las_rs::read_file(path).map_err(map_err)?;
    let mnemonics = las.curve_mnemonics();
    // Index = the depth curve. `las_rs` recognizes the common `DEPT`; when it
    // doesn't (e.g. Petrel core logs name it `DEPTH`), fall back to the LAS
    // convention that the **first** curve is the depth md.
    let md = match las.index() {
        Some(ix) => ix.to_vec(),
        None => {
            let first = mnemonics
                .first()
                .ok_or_else(|| GeoError::Parse("LAS: no md/depth curve".into()))?;
            las.curve_data(first)
                .map(<[f64]>::to_vec)
                .ok_or_else(|| GeoError::Parse("LAS: no md/depth curve".into()))?
        }
    };
    let mut curves = Vec::with_capacity(mnemonics.len().saturating_sub(1));
    for m in mnemonics.iter().skip(1) {
        let values = las.curve_data(m).map(<[f64]>::to_vec).unwrap_or_default();
        let unit = las
            .get_curve(m)
            .map(|c| c.header.unit.clone())
            .unwrap_or_default();
        curves.push(LogCurveData {
            mnemonic: (*m).to_string(),
            unit,
            values,
        });
    }
    Ok(LogData { md, curves })
}

// ---- LAS 3.0 contained fallback parser -------------------------------------

/// The data delimiter declared by a LAS 3.0 `~Version` `DLM` record.
#[derive(Clone, Copy)]
enum Delim {
    Space,
    Comma,
    Tab,
}

/// A raw LAS section: the word following `~` plus its body lines (comments kept;
/// filtered at parse time).
struct Section<'a> {
    head: String,
    lines: Vec<&'a str>,
}

/// Section role, from the word after `~` (LAS 3.0 uses full section names like
/// `Log_Definition`/`Log_Data`, not just single-letter codes).
#[derive(PartialEq)]
enum Class {
    Version,
    Well,
    Defs,
    Data,
    Other,
}

fn classify(head: &str) -> Class {
    let h = head.to_ascii_uppercase();
    // Data first so `Core_Data`/`Log_Data` (which also start with C/…) resolve
    // as data, not definitions.
    if h == "A" || h == "ASCII" || h.ends_with("_DATA") {
        Class::Data
    } else if h.starts_with('V') {
        Class::Version
    } else if h.starts_with('W') {
        Class::Well
    } else if h == "C" || h.starts_with("CURVE") || h.ends_with("_DEFINITION") {
        Class::Defs
    } else {
        Class::Other // Parameter, Other, …
    }
}

/// Split a LAS body into `~`-delimited sections (blank lines dropped; `#`
/// comment lines kept in the body and filtered per-section at parse time).
fn split_sections(text: &str) -> Vec<Section<'_>> {
    let mut secs: Vec<Section> = Vec::new();
    let mut cur: Option<Section> = None;
    for line in text.lines() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        if let Some(rest) = t.strip_prefix('~') {
            if let Some(s) = cur.take() {
                secs.push(s);
            }
            let head = rest
                .split(|c: char| c.is_whitespace() || c == '|')
                .next()
                .unwrap_or("")
                .to_string();
            cur = Some(Section {
                head,
                lines: Vec::new(),
            });
        } else if let Some(s) = cur.as_mut() {
            s.lines.push(t);
        }
    }
    if let Some(s) = cur {
        secs.push(s);
    }
    secs
}

/// The DATA field of a LAS header line `MNEM.UNIT DATA : DESC` — the last
/// whitespace token before the `:` (unit-agnostic; for `VERS`/`NULL`/`DLM`/
/// `WRAP` the unit is empty so this is exactly the value).
fn field_data(line: &str) -> Option<String> {
    let (_mnem, rest) = line.split_once('.')?;
    let before_colon = rest.split(':').next().unwrap_or(rest);
    before_colon.split_whitespace().last().map(str::to_string)
}

/// The value of header record `key` (matched case-insensitively on the mnemonic
/// before the `.`) within a section, if present.
fn find_field(sec: &Section, key: &str) -> Option<String> {
    sec.lines
        .iter()
        .filter(|l| !l.starts_with('#'))
        .find(|l| {
            l.split_once('.')
                .map(|(m, _)| m.trim().eq_ignore_ascii_case(key))
                .unwrap_or(false)
        })
        .and_then(|l| field_data(l))
}

/// Detect the LAS version from the `~Version` `VERS` record, if present.
fn detect_version(secs: &[Section]) -> Option<f64> {
    let v = secs.iter().find(|s| classify(&s.head) == Class::Version)?;
    find_field(v, "VERS")?.parse::<f64>().ok()
}

/// Parse a LAS 3.0 body into curves via the contained delimiter-aware reader.
fn parse_las3(secs: &[Section]) -> Result<LogData> {
    let version = secs.iter().find(|s| classify(&s.head) == Class::Version);
    let delim = version
        .and_then(|s| find_field(s, "DLM"))
        .map(|d| match d.to_ascii_uppercase().as_str() {
            "COMMA" => Delim::Comma,
            "TAB" => Delim::Tab,
            _ => Delim::Space,
        })
        .unwrap_or(Delim::Space);
    // Wrapped 3.0 data (one depth spanning several physical lines) is not
    // supported by this contained reader — fail loudly rather than mis-align.
    if let Some(w) = version.and_then(|s| find_field(s, "WRAP")) {
        if w.eq_ignore_ascii_case("YES") {
            return Err(GeoError::Format(
                "LAS 3.0: wrapped data (WRAP YES) is not supported".into(),
            ));
        }
    }
    let null = secs
        .iter()
        .find(|s| classify(&s.head) == Class::Well)
        .and_then(|s| find_field(s, "NULL"))
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(-999.25);

    // Walk sections in order, pairing the most recent definition section with
    // the first following data section (handles `~Curve`+`~ASCII` and
    // `~Log_Definition`+`~Log_Data`). A `~Parameter_Data` section is ignored.
    let mut last_defs: Option<Vec<(String, String)>> = None;
    for s in secs {
        match classify(&s.head) {
            Class::Defs => last_defs = Some(parse_defs(&s.lines)),
            Class::Data if s.head.to_ascii_uppercase().starts_with("PARAMETER") => {}
            Class::Data => {
                if let Some(defs) = &last_defs {
                    return build_curves(defs, &s.lines, delim, null);
                }
            }
            _ => {}
        }
    }
    Err(GeoError::Format(
        "LAS 3.0: no curve-definition + data section pair found".into(),
    ))
}

/// Parse a definition section's lines into `(mnemonic, unit)` pairs.
fn parse_defs(lines: &[&str]) -> Vec<(String, String)> {
    lines
        .iter()
        .filter(|l| !l.starts_with('#'))
        .filter_map(|l| {
            let (m, rest) = l.split_once('.')?;
            let mnem = m.trim();
            if mnem.is_empty() {
                return None;
            }
            let unit: String = rest
                .trim_start()
                .chars()
                .take_while(|c| !c.is_whitespace() && *c != ':')
                .collect();
            Some((mnem.to_string(), unit))
        })
        .collect()
}

/// Split a data line into fields by the declared delimiter.
fn split_line(line: &str, delim: Delim) -> Vec<&str> {
    match delim {
        Delim::Space => line.split_whitespace().collect(),
        Delim::Comma => line.split(',').collect(),
        Delim::Tab => line.split('\t').collect(),
    }
}

/// Parse one token to `f64`, mapping the NULL sentinel and blanks/non-numerics
/// to `NaN` (the undefined convention).
fn parse_num(tok: &str, null: f64) -> f64 {
    let t = tok.trim();
    match t.parse::<f64>() {
        Ok(v) if v == null => f64::NAN,
        Ok(v) => v,
        Err(_) => f64::NAN,
    }
}

/// Build [`LogData`] from definitions + data rows. Ragged rows are NaN-padded
/// to the curve count.
fn build_curves(
    defs: &[(String, String)],
    lines: &[&str],
    delim: Delim,
    null: f64,
) -> Result<LogData> {
    let ncol = defs.len();
    if ncol == 0 {
        return Err(GeoError::Format(
            "LAS 3.0: curve section defined no curves".into(),
        ));
    }
    let mut cols: Vec<Vec<f64>> = vec![Vec::new(); ncol];
    for l in lines {
        if l.starts_with('#') {
            continue;
        }
        let toks = split_line(l, delim);
        if toks.iter().all(|t| t.trim().is_empty()) {
            continue;
        }
        for (i, col) in cols.iter_mut().enumerate() {
            col.push(toks.get(i).map(|t| parse_num(t, null)).unwrap_or(f64::NAN));
        }
    }
    // Move each accumulated column out (no clone): col 0 is the md, the rest
    // pair with their definition.
    let mut cols = cols.into_iter();
    let md = cols.next().expect("ncol >= 1 checked above");
    let curves = cols
        .enumerate()
        .map(|(j, values)| {
            let i = j + 1;
            LogCurveData {
                mnemonic: defs[i].0.clone(),
                unit: defs[i].1.clone(),
                values,
            }
        })
        .collect();
    Ok(LogData { md, curves })
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
    fn falls_back_to_depth_index_when_not_dept() {
        // Petrel core logs name the md `DEPTH`, not the LAS-standard `DEPT`.
        let body = "\
~Version
 VERS. 2.0 :
 WRAP. NO :
~Well
 STRT.M 100.0 :
 STOP.M 120.0 :
 STEP.M 10.0 :
 NULL. -999.25 :
~Curve
 DEPTH.M : Depth
 CPOR.pu : core porosity
~ASCII
100.0 19.0
110.0 21.0
120.0 18.0
";
        let p = write("petekio_depth_index.las", body);
        let c = load(&p).unwrap();
        assert_eq!(c.md, vec![100.0, 110.0, 120.0]);
        assert_eq!(c.curves.len(), 1);
        assert_eq!(c.curves[0].mnemonic, "CPOR");
        assert_eq!(c.curves[0].values, vec![19.0, 21.0, 18.0]);
    }

    #[test]
    fn reads_las3_comma_delimited_core() {
        // Synthetic LAS 3.0: VERS 3.0, DLM COMMA, `~Log_Definition`/`~Log_Data`
        // (the naming las_rs 0.2 drops → 0 curves). NULL -999.25 → NaN.
        let body = "\
~Version
 VERS.   3.0 : CWLS LOG ASCII STANDARD - VERSION 3.0
 WRAP.   NO  :
 DLM.    COMMA :
~Well
 STRT.M  1205.0 :
 STOP.M  1215.0 :
 NULL.   -999.25 :
~Log_Definition
 DEPTH.M    : Depth
 CPOR.pu    : core porosity
 CKH.mD     : core permeability
~Log_Data | Log_Definition
1205.0, 19.5, 120.0
1210.0, 21.0, -999.25
1215.0, 18.0, 95.5
";
        let p = write("petekio_las3_core.las", body);
        let c = load(&p).unwrap();
        assert_eq!(c.md, vec![1205.0, 1210.0, 1215.0]);
        assert_eq!(c.curves.len(), 2);
        assert_eq!(c.curves[0].mnemonic, "CPOR");
        assert_eq!(c.curves[0].unit, "pu");
        assert_eq!(c.curves[0].values, vec![19.5, 21.0, 18.0]);
        assert_eq!(c.curves[1].mnemonic, "CKH");
        assert_eq!(c.curves[1].values[0], 120.0);
        assert!(c.curves[1].values[1].is_nan()); // -999.25 → NaN
        assert_eq!(c.curves[1].values[2], 95.5);
    }

    #[test]
    fn wrapped_las3_is_typed_format_error() {
        let body = "\
~Version
 VERS. 3.0 :
 WRAP. YES :
 DLM.  SPACE :
~Well
 NULL. -999.25 :
~Log_Definition
 DEPTH.M :
 GR.GAPI :
~Log_Data
1000.0
  55.0
";
        let p = write("petekio_las3_wrap.las", body);
        let err = load(&p).unwrap_err();
        assert!(matches!(err, GeoError::Format(_)), "got {err:?}");
        assert!(format!("{err}").contains("LAS 3.0"));
    }
}
