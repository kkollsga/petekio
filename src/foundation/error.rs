//! The crate-wide error type and `Result` alias.

/// The single error type returned across petekIO. Readers validate on load and
/// return a typed `GeoError`, never a panic.
#[derive(thiserror::Error, Debug)]
pub enum GeoError {
    /// Underlying I/O failure (file open/read/write).
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// A file or value could not be parsed.
    #[error("parse error: {0}")]
    Parse(String),

    /// Two operands had incompatible geometry (e.g. surface ↔ surface math on
    /// differing grids — resample first).
    #[error("geometry mismatch: {0}")]
    GeometryMismatch(String),

    /// A named item (surface, well, attribute, …) was not found.
    #[error("not found: {0}")]
    NotFound(String),

    /// An index / coordinate / depth was outside the valid range.
    #[error("out of range: {0}")]
    OutOfRange(String),

    /// A unit was missing or inconsistent.
    #[error("unit error: {0}")]
    Unit(String),
}

/// Convenience alias: `Result<T> = std::result::Result<T, GeoError>`.
pub type Result<T> = std::result::Result<T, GeoError>;
