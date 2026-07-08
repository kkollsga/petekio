//! Canonical imported point payload.
//!
//! File readers normalize their format-specific rows into `PointData`; `core`
//! then turns it into the domain-facing `PointSet`. This keeps reader quirks
//! such as Petrel `column`/`row` topology in one simple shape.

use crate::foundation::{GeoError, Result};
use indexmap::IndexMap;

/// Imported XYZ coordinates plus named `f64` attribute columns, aligned 1:1.
#[derive(Debug)]
pub(crate) struct PointData {
    pub(crate) coords: Vec<[f64; 3]>,
    pub(crate) attrs: IndexMap<String, Vec<f64>>,
}

impl PointData {
    pub(crate) fn new(coords: Vec<[f64; 3]>, attrs: IndexMap<String, Vec<f64>>) -> Result<Self> {
        for (name, col) in &attrs {
            if col.len() != coords.len() {
                return Err(GeoError::Parse(format!(
                    "point attribute '{name}' has {} rows, expected {}",
                    col.len(),
                    coords.len()
                )));
            }
        }
        Ok(Self { coords, attrs })
    }

    pub(crate) fn from_coords(coords: Vec<[f64; 3]>) -> Self {
        Self {
            coords,
            attrs: IndexMap::new(),
        }
    }

    pub(crate) fn attr_any(&self, names: &[&str]) -> Option<&[f64]> {
        self.attrs.iter().find_map(|(key, values)| {
            let normalized = normalize_attr_name(key);
            names
                .iter()
                .any(|name| normalized == *name)
                .then_some(values.as_slice())
        })
    }

    pub(crate) fn with_topology_from_ordered_subset(
        mut self,
        topology: &PointData,
        tolerance: f64,
    ) -> Result<Self> {
        let columns = topology.attr_any(&["column", "col"]).ok_or_else(|| {
            GeoError::GeometryInference(
                "topology point set does not contain a column attribute".into(),
            )
        })?;
        let rows = topology.attr_any(&["row"]).ok_or_else(|| {
            GeoError::GeometryInference(
                "topology point set does not contain a row attribute".into(),
            )
        })?;

        let mut cursor = 0usize;
        let mut out_columns = Vec::with_capacity(self.coords.len());
        let mut out_rows = Vec::with_capacity(self.coords.len());
        for point in &self.coords {
            let mut found = None;
            while cursor < topology.coords.len() {
                if xyz_close(*point, topology.coords[cursor], tolerance) {
                    found = Some(cursor);
                    cursor += 1;
                    break;
                }
                cursor += 1;
            }
            let Some(idx) = found else {
                return Err(GeoError::GeometryInference(
                    "IRAP points are not an ordered subset of the topology point export".into(),
                ));
            };
            out_columns.push(columns[idx]);
            out_rows.push(rows[idx]);
        }

        self.attrs.insert("column".to_string(), out_columns);
        self.attrs.insert("row".to_string(), out_rows);
        Ok(self)
    }

    pub(crate) fn into_parts(self) -> (Vec<[f64; 3]>, IndexMap<String, Vec<f64>>) {
        (self.coords, self.attrs)
    }
}

pub(crate) fn normalize_attr_name(name: &str) -> String {
    name.trim().to_ascii_lowercase().replace([' ', '-'], "_")
}

fn xyz_close(a: [f64; 3], b: [f64; 3], tolerance: f64) -> bool {
    (a[0] - b[0]).abs() <= tolerance
        && (a[1] - b[1]).abs() <= tolerance
        && (a[2] - b[2]).abs() <= tolerance
}
