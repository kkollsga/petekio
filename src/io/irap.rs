//! IRAP classic (ROXAR text/ASCII) surface reader + writer — petekIO's first
//! format. Whitespace-delimited, free-format; mirrors the layout xtgeo reads.
//!
//! Header = 19 leading tokens: `-996 NROW XINC YINC | XMIN XMAX YMIN YMAX |
//! NCOL ROTATION X0 Y0 | 0×7`. **NROW is token 1, NCOL is token 8** (not
//! adjacent). Undefined = `9999900.0`. Data is **column-major, x-fastest**
//! (flat index `k → i = k % ncol, j = k / ncol`); the first value is the origin
//! node `(X0, Y0)`. A negative `YINC` means `yflip`.

use crate::foundation::{GeoError, GridGeometry, Result};
use ndarray::{Array2, ShapeBuilder};
use std::path::Path;

/// The undefined-value sentinel for IRAP classic ASCII. On read, anything `>=`
/// this maps to `NaN`; on write, `NaN` maps to this.
const UNDEF_IRAP_ASCII: f64 = 9999900.0;

fn next_f64<'a>(it: &mut impl Iterator<Item = &'a str>, what: &str) -> Result<f64> {
    let t = it
        .next()
        .ok_or_else(|| GeoError::Parse(format!("IRAP classic: missing {what}")))?;
    t.parse::<f64>()
        .map_err(|e| GeoError::Parse(format!("IRAP classic: bad {what} '{t}': {e}")))
}

fn next_usize<'a>(it: &mut impl Iterator<Item = &'a str>, what: &str) -> Result<usize> {
    let t = it
        .next()
        .ok_or_else(|| GeoError::Parse(format!("IRAP classic: missing {what}")))?;
    t.parse::<usize>()
        .map_err(|e| GeoError::Parse(format!("IRAP classic: bad {what} '{t}': {e}")))
}

/// Read an IRAP-classic file into a geometry + value grid.
pub fn load_irap_classic(path: &Path) -> Result<(GridGeometry, Array2<f64>)> {
    // Latin-1 decode (permissive) like every other reader — a Petrel export with
    // a stray non-UTF-8 byte must not abort the parse.
    let text = crate::io::decode_latin1(&std::fs::read(path)?);
    parse_irap_classic(&text)
}

fn parse_irap_classic(text: &str) -> Result<(GridGeometry, Array2<f64>)> {
    let mut it = text.split_whitespace();

    let id = {
        let t = it
            .next()
            .ok_or_else(|| GeoError::Parse("IRAP classic: empty file".into()))?;
        t.parse::<i64>()
            .map_err(|e| GeoError::Parse(format!("IRAP classic: bad id flag '{t}': {e}")))?
    };
    if id != -996 {
        return Err(GeoError::Parse(format!(
            "IRAP classic: expected id -996, got {id}"
        )));
    }

    let nrow = next_usize(&mut it, "nrow")?;
    let xinc = next_f64(&mut it, "xinc")?;
    let yinc = next_f64(&mut it, "yinc")?;
    let _xmin = next_f64(&mut it, "xmin")?;
    let _xmax = next_f64(&mut it, "xmax")?; // redundant; validated by xtgeo, ignored here
    let _ymin = next_f64(&mut it, "ymin")?;
    let _ymax = next_f64(&mut it, "ymax")?;
    let ncol = next_usize(&mut it, "ncol")?;
    let rotation = next_f64(&mut it, "rotation")?;
    let x0 = next_f64(&mut it, "x0")?;
    let y0 = next_f64(&mut it, "y0")?;
    for _ in 0..7 {
        let _ = next_f64(&mut it, "reserved value")?;
    }

    let geom = GridGeometry {
        xori: x0,
        yori: y0,
        xinc: xinc.abs(),
        yinc: yinc.abs(),
        ncol,
        nrow,
        rotation_deg: rotation,
        yflip: yinc < 0.0,
    };

    let n = ncol
        .checked_mul(nrow)
        .ok_or_else(|| GeoError::Parse("IRAP classic: ncol*nrow overflows".into()))?;
    let mut data = Vec::with_capacity(n);
    for _ in 0..n {
        let v = next_f64(&mut it, "grid value")?;
        data.push(if v >= UNDEF_IRAP_ASCII { f64::NAN } else { v });
    }

    // Stream is column-major (x-fastest): k = i + j*ncol. Fortran-order shape
    // (ncol, nrow) reproduces exactly that, with logical indexing values[[i,j]].
    let values = Array2::from_shape_vec((ncol, nrow).f(), data)
        .map_err(|e| GeoError::Parse(format!("IRAP classic: shape error: {e}")))?;
    Ok((geom, values))
}

/// Write a geometry + value grid as IRAP-classic ASCII (round-trips
/// [`load_irap_classic`]). Values use Rust's shortest round-trippable float
/// formatting; `NaN` is written as the undefined sentinel.
pub fn save_irap_classic(path: &Path, geom: &GridGeometry, values: &Array2<f64>) -> Result<()> {
    if values.dim() != (geom.ncol, geom.nrow) {
        return Err(GeoError::GeometryMismatch(format!(
            "save_irap_classic: values shape {:?} != grid (ncol={}, nrow={})",
            values.dim(),
            geom.ncol,
            geom.nrow
        )));
    }
    let yinc_signed = geom.yinc * geom.yflip_factor();
    // Nominal axis extents — redundant in the format (ignored on read).
    let xmax = geom.xori + (geom.ncol.saturating_sub(1)) as f64 * geom.xinc;
    let ymax = geom.yori + (geom.nrow.saturating_sub(1)) as f64 * geom.yinc;

    let mut lines = vec![
        format!("-996 {} {} {}", geom.nrow, geom.xinc, yinc_signed),
        format!("{} {} {} {}", geom.xori, xmax, geom.yori, ymax),
        format!(
            "{} {} {} {}",
            geom.ncol, geom.rotation_deg, geom.xori, geom.yori
        ),
        "0  0  0  0  0  0  0".to_string(),
    ];

    // Emit values column-major, x-fastest, 6 per line.
    let mut tokens = Vec::with_capacity(geom.ncol * geom.nrow);
    for j in 0..geom.nrow {
        for i in 0..geom.ncol {
            let v = values[[i, j]];
            tokens.push(if v.is_nan() {
                UNDEF_IRAP_ASCII.to_string()
            } else {
                v.to_string()
            });
        }
    }
    for chunk in tokens.chunks(6) {
        lines.push(chunk.join(" "));
    }

    let mut out = lines.join("\n");
    out.push('\n');
    std::fs::write(path, out)?;
    Ok(())
}
