//! Canonical imported polygon payload.
//!
//! Vector and line readers normalize their format-specific geometry into rings
//! of `[x, y, z]`; `core` then turns those rings into the domain-facing
//! `PolygonSet`.

/// Imported polygon rings. Each ring is an ordered vertex list; Z is retained
/// at import time and intentionally dropped by the areal `PolygonSet` domain
/// object.
#[derive(Debug)]
pub(crate) struct PolygonData {
    pub(crate) rings: Vec<Vec<[f64; 3]>>,
}

impl PolygonData {
    pub(crate) fn from_rings(rings: Vec<Vec<[f64; 3]>>) -> Self {
        Self { rings }
    }

    pub(crate) fn into_rings(self) -> Vec<Vec<[f64; 3]>> {
        self.rings
    }
}
