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

    /// A file's shape/header was not the format the reader expected (an
    /// EarthVision grid handed to the IRAP-points reader), or a recognised but
    /// unsupported variant (wrapped LAS 3.0). Names the detected/declared format
    /// so the caller can route to the right reader instead of getting silent
    /// wrong data.
    #[error("unsupported format: {0}")]
    Format(String),

    /// Two operands had incompatible geometry (e.g. surface ↔ surface math on
    /// differing grids — resample first).
    #[error("geometry mismatch: {0}")]
    GeometryMismatch(String),

    /// A point cloud could not be interpreted as a regular grid geometry.
    #[error("geometry inference failed: {0}")]
    GeometryInference(String),

    /// A named item (surface, well, attribute, …) was not found.
    #[error("not found: {0}")]
    NotFound(String),

    /// An index / coordinate / depth was outside the valid range.
    #[error("out of range: {0}")]
    OutOfRange(String),

    /// A well-formed request for a capability petekIO does not (yet) support —
    /// e.g. resampling a **rotated** grid geometry, which the shared axis-aligned
    /// resample kernel does not handle. Loud and typed so the caller gets a clear
    /// "not supported" instead of a silent wrong answer.
    #[error("unsupported: {0}")]
    Unsupported(String),

    /// A unit was missing or inconsistent.
    #[error("unit error: {0}")]
    Unit(String),

    /// A failure surfaced by petekTools (e.g. the `.pproj` container framing).
    /// Composed at the seam so `?` and `source()` chains work.
    #[error(transparent)]
    Tools(#[from] petektools::AlgoError),
}

/// Convenience alias: `Result<T> = std::result::Result<T, GeoError>`.
pub type Result<T> = std::result::Result<T, GeoError>;
