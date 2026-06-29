//! LAS (Log ASCII Standard) reader — wraps `las_rs` 0.2 behind petekIO types.
//!
//! Reads the curves of a LAS 1.2/2.0/3.0 file into a shared index (MD) plus one
//! [`LasCurve`] per non-index curve. `las_rs` already maps the header NULL value
//! to `f64::NAN`, so undefined samples arrive as NaN. This module imports only
//! from `foundation`; `core::Log` builds the domain object from the raw data.

use crate::foundation::{GeoError, Result};
use std::path::Path;

/// One LAS curve: its mnemonic, unit, and samples (NULL already `f64::NAN`).
#[derive(Debug, Clone)]
pub struct LasCurve {
    pub mnemonic: String,
    pub unit: String,
    pub values: Vec<f64>,
}

/// The parsed curves of a LAS file: a shared index (the first/depth curve) and
/// every other curve.
#[derive(Debug, Clone)]
pub struct LasCurves {
    /// The index/MD curve samples.
    pub index: Vec<f64>,
    /// Non-index curves, in file order.
    pub curves: Vec<LasCurve>,
}

fn map_err(e: las_rs::LasError) -> GeoError {
    GeoError::Parse(format!("LAS: {e}"))
}

/// Read a LAS file into its index curve plus the remaining curves.
pub fn load(path: &Path) -> Result<LasCurves> {
    let las = las_rs::read_file(path).map_err(map_err)?;
    let mnemonics = las.curve_mnemonics();
    // Index = the depth curve. `las_rs` recognizes the common `DEPT`; when it
    // doesn't (e.g. Petrel core logs name it `DEPTH`), fall back to the LAS
    // convention that the **first** curve is the depth index.
    let index = match las.index() {
        Some(ix) => ix.to_vec(),
        None => {
            let first = mnemonics
                .first()
                .ok_or_else(|| GeoError::Parse("LAS: no index/depth curve".into()))?;
            las.curve_data(first)
                .map(<[f64]>::to_vec)
                .ok_or_else(|| GeoError::Parse("LAS: no index/depth curve".into()))?
        }
    };
    let mut curves = Vec::with_capacity(mnemonics.len().saturating_sub(1));
    for m in mnemonics.iter().skip(1) {
        let values = las.curve_data(m).map(<[f64]>::to_vec).unwrap_or_default();
        let unit = las
            .get_curve(m)
            .map(|c| c.header.unit.clone())
            .unwrap_or_default();
        curves.push(LasCurve {
            mnemonic: (*m).to_string(),
            unit,
            values,
        });
    }
    Ok(LasCurves { index, curves })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn falls_back_to_depth_index_when_not_dept() {
        // Petrel core logs name the index `DEPTH`, not the LAS-standard `DEPT`.
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
        let p = std::env::temp_dir().join("petekio_depth_index.las");
        std::fs::File::create(&p)
            .unwrap()
            .write_all(body.as_bytes())
            .unwrap();
        let c = load(&p).unwrap();
        assert_eq!(c.index, vec![100.0, 110.0, 120.0]);
        assert_eq!(c.curves.len(), 1);
        assert_eq!(c.curves[0].mnemonic, "CPOR");
        assert_eq!(c.curves[0].values, vec![19.0, 21.0, 18.0]);
    }
}
