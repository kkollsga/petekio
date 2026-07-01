//! `io::serial` — the pinned `bincode` codec for element payloads.
//!
//! One place fixes the wire encoding (fixed-int, little-endian) so a `.pproj`
//! stays readable regardless of bincode defaults — the same discipline kglite
//! applies. Element DTOs are positional, so a struct-layout change is a
//! `data_version` bump with a migration (see the persistence design).

use crate::foundation::{GeoError, Result};
use bincode::Options;
use serde::de::DeserializeOwned;
use serde::Serialize;

fn opts() -> impl bincode::Options {
    bincode::options()
        .with_fixint_encoding()
        .with_little_endian()
        .allow_trailing_bytes()
}

/// Encode a serializable value to bytes (the uncompressed section payload).
pub fn to_bytes<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    opts()
        .serialize(value)
        .map_err(|e| GeoError::Parse(format!("serialize: {e}")))
}

/// Decode bytes back into a value.
pub fn from_bytes<T: DeserializeOwned>(bytes: &[u8]) -> Result<T> {
    opts()
        .deserialize(bytes)
        .map_err(|e| GeoError::Parse(format!("deserialize: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_including_nan() {
        let v = vec![1.0_f64, f64::NAN, -2.5, f64::INFINITY];
        let bytes = to_bytes(&v).unwrap();
        let back: Vec<f64> = from_bytes(&bytes).unwrap();
        assert_eq!(back[0], 1.0);
        assert!(back[1].is_nan()); // NaN survives the byte round-trip
        assert_eq!(back[3], f64::INFINITY);
    }
}
