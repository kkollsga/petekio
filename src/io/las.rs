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
    let index = las
        .index()
        .map(<[f64]>::to_vec)
        .ok_or_else(|| GeoError::Parse("LAS: no index/depth curve".into()))?;
    let mnemonics = las.curve_mnemonics();
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
