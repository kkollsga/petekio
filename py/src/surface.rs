//! `Surface` — the gridded workhorse: IO, sampling, element-wise math, operator
//! overloads (scalar and surface↔surface), attribute access, statistics, and
//! volumetrics. Mirrors `petekio::Surface`.
//!
//! Numpy is out of scope, so `surface.attr["seismic"]` returns the **promoted**
//! attribute as a `Surface` (not a raw array); `surface.attr.names()` lists the
//! attribute layers.
//!
//! **Backing (the "share, don't copy" design).** A `Surface` handed back from a
//! `GeoData` project (`surface()`/`surfaces()`/`load_surface()`) is a cheap
//! **view** (`InGeo`) that re-resolves the borrowed grid by name on each call —
//! no per-access deep copy of the grid + every attribute layer. Surfaces built
//! standalone (`load_*`/`constant`/math results/promoted attributes) are `Owned`
//! (an `Arc<Surface>`, shared cheaply on clone). Mutation (`set_attr`) is
//! **copy-on-write**: an `InGeo` view detaches to an owned copy before mutating,
//! so a handed-back surface never writes back into the project — the same
//! observable semantics as the former eager deep copy, minus the copy on read.

use crate::attribute::{metadata_from_dict, metadata_to_dict};
use crate::geodata::GeoData;
use crate::geometry::{BBox, GridGeometry};
use crate::points::PolygonSet;
use crate::stats::Stats;
use crate::{parse_grid_method, to_pyerr};
use petekio::{GeoError, PolygonSet as RsPolygonSet, Surface as RsSurface};
use pyo3::exceptions::{PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict};
use std::sync::Arc;

/// Where a `Surface` wrapper reads its grid from.
enum SurfaceBacking {
    /// A standalone surface (loaded / constructed / computed), shared by `Arc`.
    Owned(Arc<RsSurface>),
    /// A view into a `GeoData` project, re-resolved by name (no copy).
    InGeo { geo: Py<GeoData>, name: String },
}

/// A regular gridded surface (IRAP/RMS model): a primary value layer plus named
/// attribute layers on the same geometry. `NaN` = undefined.
#[pyclass(name = "Surface")]
pub struct Surface {
    backing: SurfaceBacking,
    name: Option<String>,
}

impl Surface {
    /// Wrap an owned Rust surface (a hand-back that isn't tied to a project).
    pub(crate) fn wrap(inner: RsSurface) -> Surface {
        Surface {
            backing: SurfaceBacking::Owned(Arc::new(inner)),
            name: None,
        }
    }

    /// A cheap view into project `geo`'s surface `name` (no grid copy).
    pub(crate) fn view(geo: Py<GeoData>, name: String) -> Surface {
        let display = crate::leaf_name(&name);
        Surface {
            backing: SurfaceBacking::InGeo { geo, name },
            name: Some(display),
        }
    }

    /// Attach a dataset display name (the duck-typed viewer seam).
    pub(crate) fn named(mut self, name: Option<String>) -> Surface {
        self.name = name;
        self
    }

    pub(crate) fn dataset_name(&self) -> Option<String> {
        self.name.clone()
    }

    /// Resolve the borrowed Rust surface and run `f` over it.
    pub(crate) fn with<R>(&self, py: Python<'_>, f: impl FnOnce(&RsSurface) -> R) -> PyResult<R> {
        match &self.backing {
            SurfaceBacking::Owned(a) => Ok(f(a)),
            SurfaceBacking::InGeo { geo, name } => {
                let g = geo.borrow(py);
                let s = g
                    .inner
                    .surface(name)
                    .ok_or_else(|| PyValueError::new_err(format!("no surface '{name}'")))?;
                Ok(f(s))
            }
        }
    }

    /// Resolve `self` **and** `other` at once (both may be views) and run `f`.
    fn with2<R>(
        &self,
        py: Python<'_>,
        other: &Surface,
        f: impl FnOnce(&RsSurface, &RsSurface) -> R,
    ) -> PyResult<R> {
        self.with(py, |s| other.with(py, |o| f(s, o)))?
    }

    /// Get a mutable owned surface, detaching an `InGeo` view first (copy-on-write)
    /// so a mutation never writes back into the project.
    fn owned_mut(&mut self, py: Python<'_>) -> PyResult<&mut RsSurface> {
        if let SurfaceBacking::InGeo { geo, name } = &self.backing {
            let cloned = {
                let g = geo.borrow(py);
                let s = g
                    .inner
                    .surface(name)
                    .ok_or_else(|| PyValueError::new_err(format!("no surface '{name}'")))?;
                s.clone()
            };
            self.backing = SurfaceBacking::Owned(Arc::new(cloned));
        }
        match &mut self.backing {
            SurfaceBacking::Owned(a) => Ok(Arc::make_mut(a)),
            SurfaceBacking::InGeo { .. } => unreachable!("just detached to Owned"),
        }
    }
}

#[pymethods]
impl Surface {
    /// Load an IRAP-classic (ROXAR ASCII) surface from `path`.
    #[staticmethod]
    fn load_irap_classic(py: Python<'_>, path: &str) -> PyResult<Surface> {
        py.detach(|| RsSurface::load_irap_classic(path))
            .map(Surface::wrap)
            .map_err(to_pyerr)
    }

    /// Load a CPS-3 regular grid (`.CPS3grid`) surface from `path`.
    #[staticmethod]
    fn load_cps3_grid(py: Python<'_>, path: &str) -> PyResult<Surface> {
        py.detach(|| RsSurface::load_cps3_grid(path))
            .map(Surface::wrap)
            .map_err(to_pyerr)
    }

    /// A surface whose every node holds `value`, on `geom`.
    #[staticmethod]
    fn constant(geom: &GridGeometry, value: f64) -> Surface {
        Surface::wrap(RsSurface::constant(geom.inner.clone(), value))
    }

    /// Write this surface's primary layer as IRAP-classic ASCII to `path`.
    fn save_irap_classic(&self, py: Python<'_>, path: &str) -> PyResult<()> {
        self.with(py, |s| py.detach(|| s.save_irap_classic(path)))?
            .map_err(to_pyerr)
    }

    /// Human-readable operation history for this surface.
    fn history(&self, py: Python<'_>) -> PyResult<Vec<String>> {
        self.with(py, |s| s.history().to_vec())
    }

    /// Bilinear sample at world `(x, y)`; `None` outside the grid or near an
    /// undefined node.
    fn sample(&self, py: Python<'_>, x: f64, y: f64) -> PyResult<Option<f64>> {
        self.with(py, |s| s.sample(x, y))
    }

    /// Resample the primary layer onto `target` (bilinear). Kernel NaN-corner
    /// policy (nearest corner NaN → NaN, else renormalized over finite corners).
    /// Raises on a rotated source/target geometry (axis-aligned kernel only).
    fn resample(&self, py: Python<'_>, target: &GridGeometry) -> PyResult<Surface> {
        let t = target.inner.clone();
        self.with(py, |s| py.detach(|| s.resample(&t)))?
            .map(Surface::wrap)
            .map_err(to_pyerr)
    }

    /// NaN-aware square-window moving average. The original NaN mask is
    /// preserved; the returned surface is detached and primary-only.
    #[pyo3(signature = (radius = 1))]
    fn smooth(&self, py: Python<'_>, radius: usize) -> PyResult<Surface> {
        self.with(py, |s| py.detach(|| Surface::wrap(s.smooth(radius))))
    }

    /// Geological dip angle in degrees, derived in the surface's world frame.
    fn dip_angle(&self, py: Python<'_>) -> PyResult<Surface> {
        self.with(py, |s| py.detach(|| Surface::wrap(s.dip_angle())))
    }

    /// Down-dip azimuth in degrees clockwise from North. Flat nodes are NaN.
    fn dip_azimuth(&self, py: Python<'_>) -> PyResult<Surface> {
        self.with(py, |s| py.detach(|| Surface::wrap(s.dip_azimuth())))
    }

    /// Fill only original NaN nodes on this geometry using a shared petekTools
    /// gridding kernel (`nearest`, `idw`, or `min_curvature`).
    #[pyo3(signature = (method = "nearest"))]
    fn extrapolate(&self, py: Python<'_>, method: &str) -> PyResult<Surface> {
        let method = parse_grid_method(method)?;
        self.with(py, |s| py.detach(|| s.extrapolate(method)))?
            .map(Surface::wrap)
            .map_err(to_pyerr)
    }

    // ---- element-wise math (new surface) ----

    fn ln(&self, py: Python<'_>) -> PyResult<Surface> {
        self.with(py, |s| Surface::wrap(s.ln()))
    }
    fn log10(&self, py: Python<'_>) -> PyResult<Surface> {
        self.with(py, |s| Surface::wrap(s.log10()))
    }
    fn exp(&self, py: Python<'_>) -> PyResult<Surface> {
        self.with(py, |s| Surface::wrap(s.exp()))
    }
    fn sqrt(&self, py: Python<'_>) -> PyResult<Surface> {
        self.with(py, |s| Surface::wrap(s.sqrt()))
    }
    fn abs(&self, py: Python<'_>) -> PyResult<Surface> {
        self.with(py, |s| Surface::wrap(s.abs()))
    }
    fn powf(&self, py: Python<'_>, n: f64) -> PyResult<Surface> {
        self.with(py, |s| Surface::wrap(s.powf(n)))
    }
    fn clamp_min(&self, py: Python<'_>, lo: f64) -> PyResult<Surface> {
        self.with(py, |s| Surface::wrap(s.clamp_min(lo)))
    }
    fn clamp(&self, py: Python<'_>, lo: f64, hi: f64) -> PyResult<Surface> {
        self.with(py, |s| Surface::wrap(s.clamp(lo, hi)))
    }

    // ---- surface↔surface math (named forms; equal geometry required) ----

    fn plus(&self, py: Python<'_>, other: &Surface) -> PyResult<Surface> {
        self.with2(py, other, |s, o| s.plus(o))?
            .map(Surface::wrap)
            .map_err(to_pyerr)
    }
    fn minus(&self, py: Python<'_>, other: &Surface) -> PyResult<Surface> {
        self.with2(py, other, |s, o| s.minus(o))?
            .map(Surface::wrap)
            .map_err(to_pyerr)
    }
    fn times(&self, py: Python<'_>, other: &Surface) -> PyResult<Surface> {
        self.with2(py, other, |s, o| s.times(o))?
            .map(Surface::wrap)
            .map_err(to_pyerr)
    }
    fn divided_by(&self, py: Python<'_>, other: &Surface) -> PyResult<Surface> {
        self.with2(py, other, |s, o| s.divided_by(o))?
            .map(Surface::wrap)
            .map_err(to_pyerr)
    }

    /// `base - top`, optionally clamped at zero (negative thickness → 0).
    /// Works as `top.thickness(base)` and, through Python's normal unbound
    /// method protocol, as `Surface.thickness(top, base)`.
    #[pyo3(signature = (base, clamp_zero = false))]
    fn thickness(&self, py: Python<'_>, base: &Surface, clamp_zero: bool) -> PyResult<Surface> {
        self.with2(py, base, |t, b| RsSurface::thickness(t, b, clamp_zero))?
            .map(Surface::wrap)
            .map_err(to_pyerr)
    }

    // ---- operator overloads ----

    fn __add__(&self, py: Python<'_>, rhs: &Bound<'_, PyAny>) -> PyResult<Surface> {
        self.binop(py, rhs, |s, k| s + k, |a, b| a.plus(py, b))
    }
    fn __sub__(&self, py: Python<'_>, rhs: &Bound<'_, PyAny>) -> PyResult<Surface> {
        self.binop(py, rhs, |s, k| s - k, |a, b| a.minus(py, b))
    }
    fn __mul__(&self, py: Python<'_>, rhs: &Bound<'_, PyAny>) -> PyResult<Surface> {
        self.binop(py, rhs, |s, k| s * k, |a, b| a.times(py, b))
    }
    fn __truediv__(&self, py: Python<'_>, rhs: &Bound<'_, PyAny>) -> PyResult<Surface> {
        self.binop(py, rhs, |s, k| s / k, |a, b| a.divided_by(py, b))
    }

    // Reflected scalar operators (`scalar <op> surface`).
    fn __radd__(&self, py: Python<'_>, lhs: f64) -> PyResult<Surface> {
        self.with(py, |s| Surface::wrap(s + lhs))
    }
    fn __rmul__(&self, py: Python<'_>, lhs: f64) -> PyResult<Surface> {
        self.with(py, |s| Surface::wrap(s * lhs))
    }
    fn __rsub__(&self, py: Python<'_>, lhs: f64) -> PyResult<Surface> {
        // lhs - self = -(self) + lhs = self * -1 + lhs
        self.with(py, |s| Surface::wrap(&(s * -1.0) + lhs))
    }

    // ---- statistics & volumetrics ----

    /// Summary statistics over the defined nodes.
    fn stats(&self, py: Python<'_>) -> PyResult<Stats> {
        self.with(py, |s| Stats::new(s.stats()))
    }

    /// Areal extent of nodes whose value is `<= depth`.
    fn area_below(&self, py: Python<'_>, depth: f64) -> PyResult<f64> {
        self.with(py, |s| s.area_below(depth))
    }
    /// Areal extent of nodes whose value is `>= depth`.
    fn area_above(&self, py: Python<'_>, depth: f64) -> PyResult<f64> {
        self.with(py, |s| s.area_above(depth))
    }

    /// Volume between this surface and `base` (equal geometry required).
    fn volume_between(&self, py: Python<'_>, base: &Surface) -> PyResult<f64> {
        self.with2(py, base, |s, b| py.detach(|| s.volume_between(b)))?
            .map_err(to_pyerr)
    }

    /// The hypsometric curve as `[(depth, area), …]`, ascending.
    fn hypsometry(&self, py: Python<'_>) -> PyResult<Vec<(f64, f64)>> {
        self.with(py, |s| s.hypsometry())
    }

    /// The dataset name this surface was resolved under (the project lookup
    /// leaf) or derives from (e.g. `points.to_surface()` propagates the point
    /// set's name), or `None` for anonymous surfaces. Duck-typed viewer seam.
    #[getter]
    fn name(&self) -> Option<String> {
        self.name.clone()
    }

    /// Stable kind label for type dispatch without imports: `"surface"`.
    #[getter]
    fn kind(&self) -> &'static str {
        "surface"
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
    fn attr_names(&self, py: Python<'_>) -> PyResult<Vec<String>> {
        self.with(py, |s| {
            s.attr_names().iter().map(|n| n.to_string()).collect()
        })
    }

    /// Canonical durable metadata for attribute `name`.
    fn attr_metadata(&self, py: Python<'_>, name: &str) -> PyResult<Py<PyDict>> {
        let metadata = self
            .with(py, |s| s.attr_metadata(name).cloned())?
            .ok_or_else(|| {
                pyo3::exceptions::PyKeyError::new_err(format!("no attribute layer '{name}'"))
            })?;
        Ok(metadata_to_dict(py, &metadata)?.unbind())
    }

    /// Metadata of the promoted primary lane, or `None` for an ordinary
    /// primary surface.
    #[getter]
    fn primary_metadata(&self, py: Python<'_>) -> PyResult<Option<Py<PyDict>>> {
        self.with(py, |s| s.primary_metadata().cloned())?
            .map(|metadata| metadata_to_dict(py, &metadata).map(Bound::unbind))
            .transpose()
    }

    /// Set (or replace) attribute `name` from another surface's primary layer
    /// (must match this surface's geometry). Copy-on-write: an `InGeo` view
    /// detaches to an owned copy first, so the project is never mutated.
    #[pyo3(signature = (name, values, metadata = None))]
    fn set_attr(
        &mut self,
        py: Python<'_>,
        name: &str,
        values: &Surface,
        metadata: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<()> {
        let (rhs_geom, arr) = values.with(py, |v| (v.geom.clone(), v.values().clone()))?;
        let lhs_geom = self.with(py, |s| s.geom.clone())?;
        if lhs_geom != rhs_geom {
            return Err(to_pyerr(GeoError::GeometryMismatch(format!(
                "Surface::set_attr('{name}'): attribute surface must match origin, increments, \
                 node counts, rotation, and yflip"
            ))));
        }
        match metadata {
            Some(metadata) => self
                .owned_mut(py)?
                .set_attr_with_metadata(name, arr, metadata_from_dict(name, metadata)?)
                .map_err(to_pyerr),
            None => self.owned_mut(py)?.set_attr(name, arr).map_err(to_pyerr),
        }
    }

    /// `surface.thickness = values` assigns a typed surface attribute lane.
    /// Read it through `surface.attr["thickness"]`; methods with the same name
    /// remain callable on the instance and unbound through the class.
    fn __setattr__(
        &mut self,
        py: Python<'_>,
        name: &str,
        value: &Bound<'_, PyAny>,
    ) -> PyResult<()> {
        let values = value.extract::<PyRef<'_, Surface>>().map_err(|_| {
            PyTypeError::new_err(format!(
                "Surface attribute '{name}' must be assigned another Surface"
            ))
        })?;
        self.set_attr(py, name, &values, None)
    }

    // ---- shells: conversions, iso-lines, value layer ----

    /// Lift to a `StructuredMeshSurface` (free, lossless: per-node XY computed
    /// from the grid; all attribute lanes carried 1:1).
    fn to_structured_mesh(
        &self,
        py: Python<'_>,
    ) -> PyResult<crate::structured_surface::StructuredMeshSurface> {
        self.with(py, |s| py.detach(|| s.to_structured_mesh()))?
            .map(|s| {
                crate::structured_surface::StructuredMeshSurface::wrap(s).named(self.name.clone())
            })
            .map_err(to_pyerr)
    }

    /// Lift to a `TriSurface` (free, lossless: the grid quad-splits along a
    /// consistent diagonal; all attribute lanes carried 1:1 per node).
    fn to_tri_surface(&self, py: Python<'_>) -> PyResult<crate::tri_surface::TriSurface> {
        self.with(py, |s| py.detach(|| s.to_tri_surface()))?
            .map(|t| crate::tri_surface::TriSurface::wrap(t).named(self.name.clone()))
            .map_err(to_pyerr)
    }

    /// Iso-lines of a property lane: `[(level, [[(x, y), ...], ...]), ...]`.
    /// Explicit `levels` win over `interval` (levels aligned to interval
    /// multiples across the value range). NaN-aware: holes break lines.
    /// `simplify=tol` runs Douglas–Peucker on each polyline (world-unit
    /// tolerance; endpoints + ring closure preserved).
    #[pyo3(signature = (interval = None, levels = None, attr = None, simplify = None))]
    fn iso_lines(
        &self,
        py: Python<'_>,
        interval: Option<f64>,
        levels: Option<Vec<f64>>,
        attr: Option<&str>,
        simplify: Option<f64>,
    ) -> PyResult<crate::shell::PyIsoLines> {
        self.with(py, |s| {
            py.detach(|| s.iso_lines(interval, levels, attr, simplify))
        })?
        .map(crate::shell::iso_lines_py)
        .map_err(to_pyerr)
    }

    /// A property lane as the viewer's trimesh dict: `{"kind": "trimesh",
    /// "name", "nodes", "triangles", "values", "range"}` (nodes/triangles from
    /// the quad-split grid). `stride=k` returns the coarse-LOD decimation
    /// (per-block `(i,j)` striding; `range` from the full-resolution lane).
    #[pyo3(signature = (attr = None, stride = None))]
    fn value_layer(
        &self,
        py: Python<'_>,
        attr: Option<&str>,
        stride: Option<usize>,
    ) -> PyResult<Py<pyo3::types::PyDict>> {
        let layer = self
            .with(py, |s| py.detach(|| s.value_layer(attr, stride)))?
            .map_err(to_pyerr)?;
        crate::shell::value_layer_dict(py, layer)
    }

    /// Private project-view transport for an affine regular surface. It copies
    /// only compact row-major f32 lanes and u8 masks; unlike `value_layer`, it
    /// never constructs node or triangle arrays. `stride` is display-only and
    /// samples native nodes before marshaling.
    #[pyo3(signature = (attr = None, stride = 1))]
    fn _view_regular_grid(
        &self,
        py: Python<'_>,
        attr: Option<&str>,
        stride: usize,
    ) -> PyResult<Py<PyDict>> {
        if stride == 0 {
            return Err(PyValueError::new_err(
                "Surface._view_regular_grid: stride must be at least 1",
            ));
        }
        let transport = self.with(py, |surface| {
            let selected = match attr {
                Some(name) => surface.attr(name).ok_or_else(|| {
                    PyValueError::new_err(format!("no attribute layer '{name}'"))
                })?,
                None => surface.values(),
            };
            let geom = &surface.geom;
            if geom.ncol < 2 || geom.nrow < 2 {
                return Err(PyValueError::new_err(
                    "Surface._view_regular_grid: affine viewer transport requires at least 2x2 nodes",
                ));
            }
            let effective_stride = stride
                .min(geom.ncol.saturating_sub(1).max(1))
                .min(geom.nrow.saturating_sub(1).max(1));
            // Preview dimensions use ceil division and then sample evenly from
            // first through last source node. That preserves the complete
            // world footprint even when `(n-1)` is not divisible by stride,
            // so the later full-detail swap cannot change camera framing.
            let ncol = geom.ncol.saturating_sub(1).div_ceil(effective_stride) + 1;
            let nrow = geom.nrow.saturating_sub(1).div_ceil(effective_stride) + 1;
            let mut elevations = Vec::with_capacity(ncol * nrow * 4);
            let mut values = Vec::with_capacity(ncol * nrow * 4);
            let mut elevation_mask = Vec::with_capacity(ncol * nrow);
            let mut value_mask = Vec::with_capacity(ncol * nrow);
            let mut sampled_elevation_mask = Vec::with_capacity(ncol * nrow);
            for oj in 0..nrow {
                let j = (oj * (geom.nrow - 1) + (nrow - 1) / 2) / (nrow - 1);
                for oi in 0..ncol {
                    let i = (oi * (geom.ncol - 1) + (ncol - 1) / 2) / (ncol - 1);
                    let elevation = surface.values()[[i, j]];
                    let value = selected[[i, j]];
                    let elevation_finite = elevation.is_finite();
                    let value_finite = value.is_finite();
                    elevations.extend_from_slice(
                        &(if elevation_finite {
                            elevation as f32
                        } else {
                            f32::from_bits(0x7fc0_0000)
                        })
                        .to_le_bytes(),
                    );
                    values.extend_from_slice(
                        &(if value_finite {
                            value as f32
                        } else {
                            f32::from_bits(0x7fc0_0000)
                        })
                        .to_le_bytes(),
                    );
                    elevation_mask.push(u8::from(elevation_finite));
                    value_mask.push(u8::from(value_finite));
                    sampled_elevation_mask.push(elevation_finite);
                }
            }
            let triangle_count = sampled_elevation_mask
                .chunks_exact(ncol)
                .collect::<Vec<_>>()
                .windows(2)
                .map(|rows| {
                    (0..ncol.saturating_sub(1))
                        .filter(|&i| rows[0][i] && rows[0][i + 1] && rows[1][i] && rows[1][i + 1])
                        .count()
                        * 2
                })
                .sum::<usize>();
            let finite_range = |lane: &ndarray::Array2<f64>| {
                lane.iter()
                    .copied()
                    .filter(|value| value.is_finite())
                    .fold(None, |range, value| match range {
                        None => Some((value, value)),
                        Some((lo, hi)) => Some((lo.min(value), hi.max(value))),
                    })
                    .unwrap_or((0.0, 1.0))
            };
            let value_range = finite_range(selected);
            let elevation_range = finite_range(surface.values());
            let origin = geom.node_xy(0, 0);
            let end_i = geom.node_xy(geom.ncol - 1, 0);
            let end_j = geom.node_xy(0, geom.nrow - 1);
            Ok::<_, PyErr>((
                ncol,
                nrow,
                origin,
                [
                    (end_i.0 - origin.0) / (ncol - 1) as f64,
                    (end_i.1 - origin.1) / (ncol - 1) as f64,
                ],
                [
                    (end_j.0 - origin.0) / (nrow - 1) as f64,
                    (end_j.1 - origin.1) / (nrow - 1) as f64,
                ],
                elevations,
                values,
                elevation_mask,
                value_mask,
                [elevation_range.0, elevation_range.1],
                [value_range.0, value_range.1],
                triangle_count,
                effective_stride,
            ))
        })??;
        let (
            ncol,
            nrow,
            origin,
            step_i,
            step_j,
            elevations,
            values,
            elevation_mask,
            value_mask,
            elevation_range,
            value_range,
            triangle_count,
            effective_stride,
        ) = transport;
        let out = PyDict::new(py);
        out.set_item("name", attr.unwrap_or("values"))?;
        out.set_item("dimensions", [ncol, nrow])?;
        out.set_item("origin", [origin.0, origin.1])?;
        out.set_item("step_i", step_i)?;
        out.set_item("step_j", step_j)?;
        out.set_item("elevations", PyBytes::new(py, &elevations))?;
        out.set_item("values", PyBytes::new(py, &values))?;
        out.set_item("elevation_mask", PyBytes::new(py, &elevation_mask))?;
        out.set_item("value_mask", PyBytes::new(py, &value_mask))?;
        out.set_item("elevation_range", elevation_range)?;
        out.set_item("range", value_range)?;
        out.set_item("triangle_count", triangle_count)?;
        out.set_item("stride", effective_stride)?;
        Ok(out.unbind())
    }

    // ---- geometry getters ----

    /// A copy of this surface's grid geometry.
    #[getter]
    fn geometry(&self, py: Python<'_>) -> PyResult<GridGeometry> {
        self.with(py, |s| {
            GridGeometry::with_edge(s.geom.clone(), surface_edge(s))
                .named(self.name.as_ref().map(|n| format!("{n} geometry")))
        })
    }

    /// Edge polygon enclosing the surface's defined nodes.
    #[getter]
    fn edge(&self, py: Python<'_>) -> PyResult<PolygonSet> {
        self.with(py, |s| PolygonSet::owned(surface_edge(s)))
    }
    #[getter]
    fn ncol(&self, py: Python<'_>) -> PyResult<usize> {
        self.with(py, |s| s.geom.ncol)
    }
    #[getter]
    fn nrow(&self, py: Python<'_>) -> PyResult<usize> {
        self.with(py, |s| s.geom.nrow)
    }
    #[getter]
    fn rotation_deg(&self, py: Python<'_>) -> PyResult<f64> {
        self.with(py, |s| s.geom.rotation_deg)
    }
    /// Axis-aligned bounding box of the grid nodes.
    fn bbox(&self, py: Python<'_>) -> PyResult<BBox> {
        self.with(py, |s| BBox::new(s.geom.bbox()))
    }

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        self.with(py, |s| {
            format!("Surface(ncol={}, nrow={})", s.geom.ncol, s.geom.nrow)
        })
    }
}

fn surface_edge(surface: &RsSurface) -> RsPolygonSet {
    surface
        .edge()
        .unwrap_or_else(|| RsPolygonSet::from_rings(Vec::new()))
}

impl Surface {
    /// Dispatch a binary operator over a scalar `f64` or another `Surface`.
    fn binop(
        &self,
        py: Python<'_>,
        rhs: &Bound<'_, PyAny>,
        scalar: impl FnOnce(&RsSurface, f64) -> RsSurface,
        surface: impl FnOnce(&Surface, &Surface) -> PyResult<Surface>,
    ) -> PyResult<Surface> {
        if let Ok(k) = rhs.extract::<f64>() {
            self.with(py, |s| Surface::wrap(scalar(s, k)))
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
    fn __contains__(&self, py: Python<'_>, name: &str) -> PyResult<bool> {
        self.surface
            .borrow(py)
            .with(py, |s| s.attr_names().contains(&name))
    }

    /// The attribute layer names, in insertion order.
    fn names(&self, py: Python<'_>) -> PyResult<Vec<String>> {
        self.surface.borrow(py).with(py, |s| {
            s.attr_names().iter().map(|n| n.to_string()).collect()
        })
    }
}

impl AttrAccessor {
    fn promote(&self, py: Python<'_>, name: &str) -> PyResult<Surface> {
        let promoted = self
            .surface
            .borrow(py)
            .with(py, |s| s.as_attr_surface(name))?;
        match promoted {
            Some(p) => Ok(Surface::wrap(p)),
            None => Err(pyo3::exceptions::PyKeyError::new_err(format!(
                "no attribute layer '{name}'"
            ))),
        }
    }
}
