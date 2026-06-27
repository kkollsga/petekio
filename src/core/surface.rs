//! `Surface` — a regular gridded surface (the workhorse): a primary value layer
//! plus named attribute layers on the same `GridGeometry`. `NaN` = undefined.
//!
//! This module covers construction, IO, and access. Math/sampling/statistics
//! land in later phases.

use crate::foundation::{GeoError, GridGeometry, Result};
use indexmap::IndexMap;
use ndarray::Array2;
use std::path::Path;

/// A rotated regular grid (IRAP/RMS model) holding a primary value layer
/// (`values`, e.g. depth) plus named attribute layers (thickness, seismic, …)
/// on the same geometry. Undefined nodes are `NaN`.
pub struct Surface {
    /// The areal lattice. Public; `values`/`attributes` are private.
    pub geom: GridGeometry,
    values: Array2<f64>,
    attributes: IndexMap<String, Array2<f64>>,
}

impl Surface {
    /// Build a surface from a geometry and a primary value grid. The grid must
    /// be shape `(ncol, nrow)` or `GeometryMismatch` is returned.
    pub fn new(geom: GridGeometry, values: Array2<f64>) -> Result<Surface> {
        check_shape(&geom, &values, "Surface::new")?;
        Ok(Surface {
            geom,
            values,
            attributes: IndexMap::new(),
        })
    }

    /// A surface whose every node holds `value`.
    pub fn constant(geom: GridGeometry, value: f64) -> Surface {
        let values = Array2::from_elem((geom.ncol, geom.nrow), value);
        Surface {
            geom,
            values,
            attributes: IndexMap::new(),
        }
    }

    /// Load an IRAP-classic (ROXAR ASCII) surface — the first supported format.
    pub fn load_irap_classic(path: impl AsRef<Path>) -> Result<Surface> {
        let (geom, values) = crate::io::irap::load_irap_classic(path.as_ref())?;
        Ok(Surface {
            geom,
            values,
            attributes: IndexMap::new(),
        })
    }

    /// Write this surface's primary layer as IRAP-classic ASCII.
    pub fn save_irap_classic(&self, path: impl AsRef<Path>) -> Result<()> {
        crate::io::irap::save_irap_classic(path.as_ref(), &self.geom, &self.values)
    }

    /// The primary value grid, shape `(ncol, nrow)`. `NaN` = undefined.
    pub fn values(&self) -> &Array2<f64> {
        &self.values
    }

    /// A named attribute grid, if present.
    pub fn attr(&self, name: &str) -> Option<&Array2<f64>> {
        self.attributes.get(name)
    }

    /// Set (or replace) a named attribute grid. Must match the surface
    /// geometry or `GeometryMismatch` is returned.
    pub fn set_attr(&mut self, name: &str, values: Array2<f64>) -> Result<()> {
        check_shape(&self.geom, &values, "Surface::set_attr")?;
        self.attributes.insert(name.to_string(), values);
        Ok(())
    }

    /// The names of all attribute layers, in insertion order.
    pub fn attr_names(&self) -> Vec<&str> {
        self.attributes.keys().map(String::as_str).collect()
    }

    /// Promote an attribute layer to a standalone `Surface` (its primary
    /// values), so surface operations can run on it.
    pub fn as_attr_surface(&self, name: &str) -> Option<Surface> {
        self.attributes.get(name).map(|a| Surface {
            geom: self.geom.clone(),
            values: a.clone(),
            attributes: IndexMap::new(),
        })
    }
}

fn check_shape(geom: &GridGeometry, values: &Array2<f64>, ctx: &str) -> Result<()> {
    if values.dim() != (geom.ncol, geom.nrow) {
        return Err(GeoError::GeometryMismatch(format!(
            "{ctx}: values shape {:?} != grid (ncol={}, nrow={})",
            values.dim(),
            geom.ncol,
            geom.nrow
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn geom() -> GridGeometry {
        GridGeometry {
            xori: 0.0,
            yori: 0.0,
            xinc: 10.0,
            yinc: 10.0,
            ncol: 2,
            nrow: 2,
            rotation_deg: 0.0,
            yflip: false,
        }
    }

    #[test]
    fn new_rejects_wrong_shape() {
        let bad = Array2::from_elem((3, 3), 1.0);
        assert!(Surface::new(geom(), bad).is_err());
    }

    #[test]
    fn attributes_set_get_promote() {
        let mut s = Surface::constant(geom(), 1.0);
        s.set_attr("thickness", Array2::from_elem((2, 2), 5.0))
            .unwrap();
        assert_eq!(s.attr_names(), vec!["thickness"]);
        assert_eq!(s.attr("thickness").unwrap()[[0, 0]], 5.0);
        assert!(s.attr("missing").is_none());
        let promoted = s.as_attr_surface("thickness").unwrap();
        assert_eq!(promoted.values()[[1, 1]], 5.0);
        // wrong-shape attr rejected
        assert!(s.set_attr("bad", Array2::from_elem((1, 1), 0.0)).is_err());
    }
}
