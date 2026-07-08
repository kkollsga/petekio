//! `io` — format readers/writers (IRAP, ZMAP, CSV, LAS, Excel, vector). Wraps
//! external crates behind petekIO's own types. Imports only from `foundation`.

/// Decode bytes as **ISO-8859-1 / Latin-1**: each byte maps to the Unicode code
/// point of the same value. Petrel exports are Latin-1/Windows-1252, so this
/// preserves Norwegian characters (`ø`/`å`/`æ`, `0xC0–0xFF`) that UTF-8 decoding
/// would replace with `�`. (Windows-1252 differs only in `0x80–0x9F` punctuation,
/// not letters, so this is exact for names.)
pub(crate) fn decode_latin1(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| b as char).collect()
}

/// The `1.0E+30`-family undefined-node sentinel: any `|z| ≥ this` is treated as
/// null regardless of the file's *declared* null value. Shared by the grid
/// readers (CPS-3, EarthVision) whose exports use this convention.
pub(crate) const NULL_THRESHOLD: f64 = 1e29;

/// The default `1.0E+30` undefined-node value assumed by the `1.0E+30`-family
/// grid readers (CPS-3, EarthVision) before a file *declares* its own null. One
/// home for the sentinel default the readers otherwise each spelled inline.
pub(crate) const DEFAULT_NULL_1E30: f64 = 1.0e30;

/// Whether `z` is an undefined-node sentinel — either in the `1.0E+30` null
/// family (`|z| ≥ [NULL_THRESHOLD]`) or within a relative epsilon of the file's
/// declared `null` value. One home for the CPS-3 / EarthVision null test.
pub(crate) fn is_null_sentinel(z: f64, null: f64) -> bool {
    z.abs() >= NULL_THRESHOLD || (z - null).abs() <= null.abs() * 1e-9
}

/// Convert a `csv::Error` to a [`GeoError`](crate::foundation::GeoError),
/// **preserving an underlying I/O failure as `GeoError::Io`** (so `source()`
/// chains reach the OS error) rather than stringifying it into `Parse`. A
/// genuine CSV *format* problem (bad UTF-8, ragged records, …) stays `Parse`,
/// prefixed with `context`.
pub(crate) fn csv_error(context: &str, e: csv::Error) -> crate::foundation::GeoError {
    use crate::foundation::GeoError;
    if matches!(e.kind(), csv::ErrorKind::Io(_)) {
        match e.into_kind() {
            csv::ErrorKind::Io(io) => GeoError::Io(io),
            _ => unreachable!("kind() reported Io"),
        }
    } else {
        GeoError::Parse(format!("{context}: {e}"))
    }
}

pub mod container;
pub mod cps3;
pub mod crsmeta;
pub mod csv_points;
pub mod detect;
pub mod earthvision;
pub mod serial;
pub mod irap;
pub mod las;
pub(crate) mod log_data;
pub mod petrel_tops;
pub(crate) mod point_data;
pub(crate) mod polygon_data;
pub(crate) mod surface_data;
pub mod tops;
pub mod vector;
pub mod vector_write;
pub mod wellpath;
pub mod xyz;

pub(crate) use log_data::{LogCurveData, LogData};
pub(crate) use point_data::{normalize_attr_name, PointData};
pub(crate) use polygon_data::PolygonData;
pub(crate) use surface_data::SurfaceData;
