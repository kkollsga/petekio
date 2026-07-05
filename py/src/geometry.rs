//! `GridGeometry` + `BBox` — the rotatable lattice and an axis-aligned box.
//! Mirrors `petekio::{GridGeometry, BBox}`.

use petekio::{BBox as RsBBox, GridGeometry as RsGeom};
use pyo3::prelude::*;

/// An axis-aligned 2-D bounding box (read-only).
#[pyclass(name = "BBox", frozen)]
pub struct BBox {
    pub(crate) inner: RsBBox,
}

impl BBox {
    pub(crate) fn new(inner: RsBBox) -> BBox {
        BBox { inner }
    }
}

#[pymethods]
impl BBox {
    #[getter]
    fn xmin(&self) -> f64 {
        self.inner.xmin
    }
    #[getter]
    fn ymin(&self) -> f64 {
        self.inner.ymin
    }
    #[getter]
    fn xmax(&self) -> f64 {
        self.inner.xmax
    }
    #[getter]
    fn ymax(&self) -> f64 {
        self.inner.ymax
    }

    fn __repr__(&self) -> String {
        format!(
            "BBox(xmin={}, ymin={}, xmax={}, ymax={})",
            self.inner.xmin, self.inner.ymin, self.inner.xmax, self.inner.ymax
        )
    }
}

/// A regular, rotatable areal lattice (the IRAP/RMS model). Construct directly,
/// or read one back from `surface.geometry`.
#[pyclass(name = "GridGeometry")]
pub struct GridGeometry {
    pub(crate) inner: RsGeom,
}

impl GridGeometry {
    pub(crate) fn new(inner: RsGeom) -> GridGeometry {
        GridGeometry { inner }
    }
}

#[pymethods]
impl GridGeometry {
    #[new]
    #[pyo3(signature = (xori, yori, xinc, yinc, ncol, nrow, rotation_deg = 0.0, yflip = false))]
    #[allow(clippy::too_many_arguments)]
    fn py_new(
        xori: f64,
        yori: f64,
        xinc: f64,
        yinc: f64,
        ncol: usize,
        nrow: usize,
        rotation_deg: f64,
        yflip: bool,
    ) -> GridGeometry {
        GridGeometry {
            inner: RsGeom {
                xori,
                yori,
                xinc,
                yinc,
                ncol,
                nrow,
                rotation_deg,
                yflip,
            },
        }
    }

    #[getter]
    fn xori(&self) -> f64 {
        self.inner.xori
    }
    #[getter]
    fn yori(&self) -> f64 {
        self.inner.yori
    }
    #[getter]
    fn xinc(&self) -> f64 {
        self.inner.xinc
    }
    #[getter]
    fn yinc(&self) -> f64 {
        self.inner.yinc
    }
    #[getter]
    fn ncol(&self) -> usize {
        self.inner.ncol
    }
    #[getter]
    fn nrow(&self) -> usize {
        self.inner.nrow
    }
    #[getter]
    fn rotation_deg(&self) -> f64 {
        self.inner.rotation_deg
    }
    #[getter]
    fn yflip(&self) -> bool {
        self.inner.yflip
    }

    /// World `(x, y)` of node `(i, j)`.
    fn node_xy(&self, i: usize, j: usize) -> (f64, f64) {
        self.inner.node_xy(i, j)
    }

    /// Fractional node coordinates `(fi, fj)` for world `(x, y)`, or `None` for
    /// a degenerate (zero-spacing) geometry.
    fn xy_to_ij(&self, x: f64, y: f64) -> Option<(f64, f64)> {
        self.inner.xy_to_ij(x, y)
    }

    /// Axis-aligned bounding box of all nodes.
    fn bbox(&self) -> BBox {
        BBox::new(self.inner.bbox())
    }

    fn __repr__(&self) -> String {
        format!(
            "GridGeometry(ncol={}, nrow={}, xinc={}, yinc={}, rotation_deg={})",
            self.inner.ncol,
            self.inner.nrow,
            self.inner.xinc,
            self.inner.yinc,
            self.inner.rotation_deg
        )
    }
}
