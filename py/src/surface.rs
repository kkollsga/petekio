//! `Surface` — the gridded workhorse: IO, sampling, element-wise math, operator
//! overloads (scalar and surface↔surface), attribute access, statistics, and
//! volumetrics. Mirrors `petekio::Surface`.
//!
//! Numpy is out of scope, so `surface.attr["seismic"]` returns the **promoted**
//! attribute as a `Surface` (not a raw array); `surface.attr.names()` lists the
//! attribute layers.

use crate::geometry::{BBox, GridGeometry};
use crate::stats::Stats;
use crate::to_pyerr;
use petekio::Surface as RsSurface;
use pyo3::exceptions::PyTypeError;
use pyo3::prelude::*;

/// Deep-copy a Rust surface (primary layer + all attribute layers) via the
/// public API — `Surface` is not `Clone`, but every binding hand-back is owned.
pub(crate) fn clone_surface(s: &RsSurface) -> RsSurface {
    let mut out = RsSurface::new(s.geom.clone(), s.values().clone())
        .expect("surface geometry matches its own values");
    let names: Vec<String> = s.attr_names().iter().map(|n| n.to_string()).collect();
    for name in names {
        if let Some(a) = s.attr(&name) {
            out.set_attr(&name, a.clone())
                .expect("attribute matches surface geometry");
        }
    }
    out
}

/// A regular gridded surface (IRAP/RMS model): a primary value layer plus named
/// attribute layers on the same geometry. `NaN` = undefined.
#[pyclass(name = "Surface")]
pub struct Surface {
    pub(crate) inner: RsSurface,
}

impl Surface {
    pub(crate) fn wrap(inner: RsSurface) -> Surface {
        Surface { inner }
    }
}

#[pymethods]
impl Surface {
    /// Load an IRAP-classic (ROXAR ASCII) surface from `path`.
    #[staticmethod]
    fn load_irap_classic(path: &str) -> PyResult<Surface> {
        RsSurface::load_irap_classic(path)
            .map(Surface::wrap)
            .map_err(to_pyerr)
    }

    /// A surface whose every node holds `value`, on `geom`.
    #[staticmethod]
    fn constant(geom: &GridGeometry, value: f64) -> Surface {
        Surface::wrap(RsSurface::constant(geom.inner.clone(), value))
    }

    /// Write this surface's primary layer as IRAP-classic ASCII to `path`.
    fn save_irap_classic(&self, path: &str) -> PyResult<()> {
        self.inner.save_irap_classic(path).map_err(to_pyerr)
    }

    /// Bilinear sample at world `(x, y)`; `None` outside the grid or near an
    /// undefined node.
    fn sample(&self, x: f64, y: f64) -> Option<f64> {
        self.inner.sample(x, y)
    }

    /// Resample the primary layer onto `target` (bilinear, strict NaN-aware).
    fn resample(&self, target: &GridGeometry) -> Surface {
        Surface::wrap(self.inner.resample(&target.inner))
    }

    // ---- element-wise math (new surface) ----

    fn ln(&self) -> Surface {
        Surface::wrap(self.inner.ln())
    }
    fn log10(&self) -> Surface {
        Surface::wrap(self.inner.log10())
    }
    fn exp(&self) -> Surface {
        Surface::wrap(self.inner.exp())
    }
    fn sqrt(&self) -> Surface {
        Surface::wrap(self.inner.sqrt())
    }
    fn abs(&self) -> Surface {
        Surface::wrap(self.inner.abs())
    }
    fn powf(&self, n: f64) -> Surface {
        Surface::wrap(self.inner.powf(n))
    }
    fn clamp_min(&self, lo: f64) -> Surface {
        Surface::wrap(self.inner.clamp_min(lo))
    }
    fn clamp(&self, lo: f64, hi: f64) -> Surface {
        Surface::wrap(self.inner.clamp(lo, hi))
    }

    // ---- surface↔surface math (named forms; equal geometry required) ----

    fn plus(&self, other: &Surface) -> PyResult<Surface> {
        self.inner
            .plus(&other.inner)
            .map(Surface::wrap)
            .map_err(to_pyerr)
    }
    fn minus(&self, other: &Surface) -> PyResult<Surface> {
        self.inner
            .minus(&other.inner)
            .map(Surface::wrap)
            .map_err(to_pyerr)
    }
    fn times(&self, other: &Surface) -> PyResult<Surface> {
        self.inner
            .times(&other.inner)
            .map(Surface::wrap)
            .map_err(to_pyerr)
    }
    fn divided_by(&self, other: &Surface) -> PyResult<Surface> {
        self.inner
            .divided_by(&other.inner)
            .map(Surface::wrap)
            .map_err(to_pyerr)
    }

    /// `base - top`, optionally clamped at zero (negative thickness → 0).
    #[staticmethod]
    #[pyo3(signature = (top, base, clamp_zero = false))]
    fn thickness(top: &Surface, base: &Surface, clamp_zero: bool) -> PyResult<Surface> {
        RsSurface::thickness(&top.inner, &base.inner, clamp_zero)
            .map(Surface::wrap)
            .map_err(to_pyerr)
    }

    // ---- operator overloads ----

    fn __add__(&self, rhs: &Bound<'_, PyAny>) -> PyResult<Surface> {
        self.binop(rhs, |s, k| Ok(&s.inner + k), |s, o| s.plus(o))
    }
    fn __sub__(&self, rhs: &Bound<'_, PyAny>) -> PyResult<Surface> {
        self.binop(rhs, |s, k| Ok(&s.inner - k), |s, o| s.minus(o))
    }
    fn __mul__(&self, rhs: &Bound<'_, PyAny>) -> PyResult<Surface> {
        self.binop(rhs, |s, k| Ok(&s.inner * k), |s, o| s.times(o))
    }
    fn __truediv__(&self, rhs: &Bound<'_, PyAny>) -> PyResult<Surface> {
        self.binop(rhs, |s, k| Ok(&s.inner / k), |s, o| s.divided_by(o))
    }

    // Reflected scalar operators (`scalar <op> surface`).
    fn __radd__(&self, lhs: f64) -> Surface {
        Surface::wrap(&self.inner + lhs)
    }
    fn __rmul__(&self, lhs: f64) -> Surface {
        Surface::wrap(&self.inner * lhs)
    }
    fn __rsub__(&self, lhs: f64) -> Surface {
        // lhs - self = -(self) + lhs = self * -1 + lhs
        Surface::wrap(&(&self.inner * -1.0) + lhs)
    }

    // ---- statistics & volumetrics ----

    /// Summary statistics over the defined nodes.
    fn stats(&self) -> Stats {
        Stats::new(self.inner.stats())
    }

    /// Areal extent of nodes whose value is `<= depth`.
    fn area_below(&self, depth: f64) -> f64 {
        self.inner.area_below(depth)
    }
    /// Areal extent of nodes whose value is `>= depth`.
    fn area_above(&self, depth: f64) -> f64 {
        self.inner.area_above(depth)
    }

    /// Volume between this surface and `base` (equal geometry required).
    fn volume_between(&self, base: &Surface) -> PyResult<f64> {
        self.inner.volume_between(&base.inner).map_err(to_pyerr)
    }

    /// The hypsometric curve as `[(depth, area), …]`, ascending.
    fn hypsometry(&self) -> Vec<(f64, f64)> {
        self.inner.hypsometry()
    }

    // ---- attribute access ----

    /// The attribute accessor: `surface.attr["seismic"]` (or `surface.attr(name)`)
    /// returns the promoted attribute layer as a `Surface`; `.names()` lists them.
    #[getter]
    fn attr(slf: Bound<'_, Self>) -> AttrAccessor {
        AttrAccessor {
            surface: slf.unbind(),
        }
    }

    /// The names of all attribute layers, in insertion order.
    fn attr_names(&self) -> Vec<String> {
        self.inner
            .attr_names()
            .iter()
            .map(|s| s.to_string())
            .collect()
    }

    /// Set (or replace) attribute `name` from another surface's primary layer
    /// (must match this surface's geometry).
    fn set_attr(&mut self, name: &str, values: &Surface) -> PyResult<()> {
        self.inner
            .set_attr(name, values.inner.values().clone())
            .map_err(to_pyerr)
    }

    // ---- geometry getters ----

    /// A copy of this surface's grid geometry.
    #[getter]
    fn geometry(&self) -> GridGeometry {
        GridGeometry::new(self.inner.geom.clone())
    }
    #[getter]
    fn ncol(&self) -> usize {
        self.inner.geom.ncol
    }
    #[getter]
    fn nrow(&self) -> usize {
        self.inner.geom.nrow
    }
    #[getter]
    fn rotation_deg(&self) -> f64 {
        self.inner.geom.rotation_deg
    }
    /// Axis-aligned bounding box of the grid nodes.
    fn bbox(&self) -> BBox {
        BBox::new(self.inner.geom.bbox())
    }

    fn __repr__(&self) -> String {
        format!(
            "Surface(ncol={}, nrow={})",
            self.inner.geom.ncol, self.inner.geom.nrow
        )
    }
}

impl Surface {
    /// Dispatch a binary operator over a scalar `f64` or another `Surface`.
    fn binop(
        &self,
        rhs: &Bound<'_, PyAny>,
        scalar: impl FnOnce(&Surface, f64) -> PyResult<RsSurface>,
        surface: impl FnOnce(&Surface, &Surface) -> PyResult<Surface>,
    ) -> PyResult<Surface> {
        if let Ok(k) = rhs.extract::<f64>() {
            scalar(self, k).map(Surface::wrap)
        } else if let Ok(other) = rhs.extract::<PyRef<'_, Surface>>() {
            surface(self, &other)
        } else {
            Err(PyTypeError::new_err(
                "Surface operands must be a float or another Surface",
            ))
        }
    }
}

/// Accessor returned by `surface.attr`: subscript or call by attribute name to
/// promote that attribute layer to a standalone `Surface`.
#[pyclass(name = "AttrAccessor")]
pub struct AttrAccessor {
    surface: Py<Surface>,
}

#[pymethods]
impl AttrAccessor {
    /// `surface.attr["name"]` → the promoted attribute layer as a `Surface`.
    fn __getitem__(&self, py: Python<'_>, name: &str) -> PyResult<Surface> {
        self.promote(py, name)
    }

    /// `surface.attr("name")` → the promoted attribute layer as a `Surface`.
    fn __call__(&self, py: Python<'_>, name: &str) -> PyResult<Surface> {
        self.promote(py, name)
    }

    /// `name in surface.attr`.
    fn __contains__(&self, py: Python<'_>, name: &str) -> bool {
        let s = self.surface.borrow(py);
        s.inner.attr_names().contains(&name)
    }

    /// The attribute layer names, in insertion order.
    fn names(&self, py: Python<'_>) -> Vec<String> {
        let s = self.surface.borrow(py);
        s.inner.attr_names().iter().map(|n| n.to_string()).collect()
    }
}

impl AttrAccessor {
    fn promote(&self, py: Python<'_>, name: &str) -> PyResult<Surface> {
        let s = self.surface.borrow(py);
        match s.inner.as_attr_surface(name) {
            Some(promoted) => Ok(Surface::wrap(promoted)),
            None => Err(pyo3::exceptions::PyKeyError::new_err(format!(
                "no attribute layer '{name}'"
            ))),
        }
    }
}
