//! Canonical imported log payload.
//!
//! LAS readers normalize file-version quirks into `LogData`; `core` then turns
//! each curve into the domain-facing measured-depth `Log`.

/// One imported log curve: mnemonic, unit, and samples aligned to `LogData.md`.
#[derive(Debug, Clone)]
pub(crate) struct LogCurveData {
    pub(crate) mnemonic: String,
    pub(crate) unit: String,
    pub(crate) values: Vec<f64>,
}

/// Imported curves from one log file: a shared measured-depth index and every
/// non-index curve in file order.
#[derive(Debug, Clone)]
pub(crate) struct LogData {
    pub(crate) md: Vec<f64>,
    pub(crate) curves: Vec<LogCurveData>,
}
