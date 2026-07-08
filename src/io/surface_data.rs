//! Canonical imported surface payload.
//!
//! Grid readers normalize format-specific headers and null values into
//! `SurfaceData`; `core` then turns it into the domain-facing `Surface`.

use crate::foundation::{GeoError, GridGeometry, Result};
use indexmap::IndexMap;
use ndarray::Array2;

/// Imported regular-grid surface values plus optional aligned attribute grids.
#[derive(Debug)]
pub(crate) struct SurfaceData {
    pub(crate) geom: GridGeometry,
    pub(crate) values: Array2<f64>,
    pub(crate) attrs: IndexMap<String, Array2<f64>>,
}

impl SurfaceData {
    pub(crate) fn new(geom: GridGeometry, values: Array2<f64>) -> Result<Self> {
        Self::with_attrs(geom, values, IndexMap::new())
    }

    pub(crate) fn with_attrs(
        geom: GridGeometry,
        values: Array2<f64>,
        attrs: IndexMap<String, Array2<f64>>,
    ) -> Result<Self> {
        check_shape(&geom, &values, "surface values")?;
        for (name, attr) in &attrs {
            check_shape(&geom, attr, &format!("surface attribute '{name}'"))?;
        }
        Ok(Self {
            geom,
            values,
            attrs,
        })
    }

    pub(crate) fn into_parts(self) -> (GridGeometry, Array2<f64>, IndexMap<String, Array2<f64>>) {
        (self.geom, self.values, self.attrs)
    }
}

fn check_shape(geom: &GridGeometry, values: &Array2<f64>, what: &str) -> Result<()> {
    if values.dim() != (geom.ncol, geom.nrow) {
        return Err(GeoError::GeometryMismatch(format!(
            "{what} shape {:?} != grid (ncol={}, nrow={})",
            values.dim(),
            geom.ncol,
            geom.nrow
        )));
    }
    Ok(())
}
