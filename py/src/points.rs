//! `PointSet` + `PolygonSet` — scattered points (attributes, nearest, gridding)
//! and polygon rings (contains, area, clip). Mirrors `petekio::{PointSet,
//! PolygonSet}`.
//!
//! Each wrapper is either **owned** (built by a `load_*` classmethod, held in an
//! `Arc`) or a **view** into a `GeoData` collection (re-resolved by name on each
//! call). Numpy is out of scope: `attr` returns a `list[float]`, not an array.

use crate::geodata::GeoData;
use crate::geometry::{BBox, GridGeometry};
use crate::stats::Stats;
use crate::surface::Surface;
use crate::{parse_grid_method, to_pyerr};
use petekio::{PointSet as RsPointSet, PolygonSet as RsPolygonSet};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use std::sync::Arc;

/// Where a `PointSet` wrapper reads its data from.
enum PointBacking {
    Owned(Arc<RsPointSet>),
    InGeo { geo: Py<GeoData>, name: String },
}

/// Scattered 3-D points with named `f64` attribute columns.
#[pyclass(name = "PointSet")]
pub struct PointSet {
    backing: PointBacking,
}

impl PointSet {
    pub(crate) fn owned(inner: RsPointSet) -> PointSet {
        PointSet {
            backing: PointBacking::Owned(Arc::new(inner)),
        }
    }

    pub(crate) fn view(geo: Py<GeoData>, name: String) -> PointSet {
        PointSet {
            backing: PointBacking::InGeo { geo, name },
        }
    }

    fn with<R>(&self, py: Python<'_>, f: impl FnOnce(&RsPointSet) -> PyResult<R>) -> PyResult<R> {
        match &self.backing {
            PointBacking::Owned(a) => f(a.as_ref()),
            PointBacking::InGeo { geo, name } => {
                let g = geo.borrow(py);
                let p = g
                    .inner
                    .points(name)
                    .ok_or_else(|| PyValueError::new_err(format!("no point set '{name}'")))?;
                f(p)
            }
        }
    }
}

#[pymethods]
impl PointSet {
    /// Load a headered CSV, taking X/Y/Z from the named columns; other numeric
    /// columns become attributes.
    #[staticmethod]
    #[pyo3(signature = (path, x = "x", y = "y", z = "z"))]
    fn load_csv(path: &str, x: &str, y: &str, z: &str) -> PyResult<PointSet> {
        RsPointSet::load_csv(path, x, y, z)
            .map(PointSet::owned)
            .map_err(to_pyerr)
    }

    /// Load point features from a GeoJSON file (numeric properties → attributes).
    #[staticmethod]
    fn load_geojson(path: &str) -> PyResult<PointSet> {
        RsPointSet::load_geojson(path)
            .map(PointSet::owned)
            .map_err(to_pyerr)
    }

    /// Load scattered points from an IRAP/RMS plain `X Y Z` file.
    #[staticmethod]
    fn load_irap_points(path: &str) -> PyResult<PointSet> {
        RsPointSet::load_irap_points(path)
            .map(PointSet::owned)
            .map_err(to_pyerr)
    }

    fn __len__(&self, py: Python<'_>) -> PyResult<usize> {
        self.with(py, |p| Ok(p.len()))
    }

    /// A named attribute column as a `list[float]`, or `None` if absent.
    fn attr(&self, py: Python<'_>, name: &str) -> PyResult<Option<Vec<f64>>> {
        self.with(py, |p| Ok(p.attr(name).map(|c| c.to_vec())))
    }

    /// NaN-skipping statistics over a named attribute column, or `None`.
    fn stats(&self, py: Python<'_>, attr: &str) -> PyResult<Option<Stats>> {
        self.with(py, |p| Ok(p.stats(attr).map(Stats::new)))
    }

    /// Axis-aligned bounding box of the points' XY.
    fn bbox(&self, py: Python<'_>) -> PyResult<BBox> {
        self.with(py, |p| Ok(BBox::new(p.bbox())))
    }

    /// Index of the areally-nearest point to `(x, y)`, or `None` if empty.
    fn nearest(&self, py: Python<'_>, x: f64, y: f64) -> PyResult<Option<usize>> {
        self.with(py, |p| Ok(p.nearest(x, y)))
    }

    /// Grid the points' Z values onto `geom` using `method` (`"nearest"`,
    /// `"idw"`, or `"min_curvature"`), returning a new `Surface`.
    #[pyo3(signature = (geom, method = "idw"))]
    fn to_surface(&self, py: Python<'_>, geom: &GridGeometry, method: &str) -> PyResult<Surface> {
        let gm = parse_grid_method(method)?;
        self.with(py, |p| {
            p.to_surface(geom.inner.clone(), gm)
                .map(Surface::wrap)
                .map_err(to_pyerr)
        })
    }

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        self.with(py, |p| Ok(format!("PointSet(len={})", p.len())))
    }
}

/// Where a `PolygonSet` wrapper reads its data from.
enum PolyBacking {
    Owned(Arc<RsPolygonSet>),
    InGeo { geo: Py<GeoData>, name: String },
}

/// A collection of polygon rings (boundaries, faults, contacts).
#[pyclass(name = "PolygonSet")]
pub struct PolygonSet {
    backing: PolyBacking,
}

impl PolygonSet {
    pub(crate) fn owned(inner: RsPolygonSet) -> PolygonSet {
        PolygonSet {
            backing: PolyBacking::Owned(Arc::new(inner)),
        }
    }

    pub(crate) fn view(geo: Py<GeoData>, name: String) -> PolygonSet {
        PolygonSet {
            backing: PolyBacking::InGeo { geo, name },
        }
    }

    fn with<R>(&self, py: Python<'_>, f: impl FnOnce(&RsPolygonSet) -> PyResult<R>) -> PyResult<R> {
        match &self.backing {
            PolyBacking::Owned(a) => f(a.as_ref()),
            PolyBacking::InGeo { geo, name } => {
                let g = geo.borrow(py);
                let p = g
                    .inner
                    .polygons(name)
                    .ok_or_else(|| PyValueError::new_err(format!("no polygon set '{name}'")))?;
                f(p)
            }
        }
    }
}

#[pymethods]
impl PolygonSet {
    /// Load polygons from a GeoJSON file.
    #[staticmethod]
    fn load_geojson(path: &str) -> PyResult<PolygonSet> {
        RsPolygonSet::load_geojson(path)
            .map(PolygonSet::owned)
            .map_err(to_pyerr)
    }

    /// Load polygons from an IRAP/RMS plain `X Y Z` file (`999.0` separators).
    #[staticmethod]
    fn load_irap_polygons(path: &str) -> PyResult<PolygonSet> {
        RsPolygonSet::load_irap_polygons(path)
            .map(PolygonSet::owned)
            .map_err(to_pyerr)
    }

    /// Load polygons from an ESRI shapefile (pass the `.shp` path).
    #[staticmethod]
    fn load_shapefile(path: &str) -> PyResult<PolygonSet> {
        RsPolygonSet::load_shapefile(path)
            .map(PolygonSet::owned)
            .map_err(to_pyerr)
    }

    /// Whether `(x, y)` is inside any polygon (boundary-exclusive).
    fn contains(&self, py: Python<'_>, x: f64, y: f64) -> PyResult<bool> {
        self.with(py, |p| Ok(p.contains(x, y)))
    }

    /// Total unsigned area of all polygons.
    fn area(&self, py: Python<'_>) -> PyResult<f64> {
        self.with(py, |p| Ok(p.area()))
    }

    /// Axis-aligned bounding box over all polygons.
    fn bbox(&self, py: Python<'_>) -> PyResult<BBox> {
        self.with(py, |p| Ok(BBox::new(p.bbox())))
    }

    /// A copy of `surface` with every node outside all polygons masked to `NaN`.
    fn clip(&self, py: Python<'_>, surface: &Surface) -> PyResult<Surface> {
        self.with(py, |p| Ok(Surface::wrap(p.clip(&surface.inner))))
    }
}
