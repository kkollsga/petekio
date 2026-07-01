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

pub mod container;
pub mod csv_points;
pub mod irap;
pub mod las;
pub mod petrel_tops;
pub mod tops;
pub mod vector;
pub mod wellpath;
pub mod xyz;
