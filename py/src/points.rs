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
use petekio::{GeometryEdge, PointSet as RsPointSet, PolygonSet as RsPolygonSet};
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
    fn load_csv(py: Python<'_>, path: &str, x: &str, y: &str, z: &str) -> PyResult<PointSet> {
        py.detach(|| RsPointSet::load_csv(path, x, y, z))
            .map(PointSet::owned)
            .map_err(to_pyerr)
    }

    /// Load point features from a GeoJSON file (numeric properties → attributes).
    #[staticmethod]
    fn load_geojson(py: Python<'_>, path: &str) -> PyResult<PointSet> {
        py.detach(|| RsPointSet::load_geojson(path))
            .map(PointSet::owned)
            .map_err(to_pyerr)
    }

    /// Load scattered points from an IRAP/RMS plain `X Y Z` file (also the
    /// `.IrapClassicPoints` content). Rejects a foreign header with `Format`.
    #[staticmethod]
    fn load_irap_points(py: Python<'_>, path: &str) -> PyResult<PointSet> {
        py.detach(|| RsPointSet::load_irap_points(path))
            .map(PointSet::owned)
            .map_err(to_pyerr)
    }

    /// Load scattered points from an EarthVision grid ASCII file
    /// (`.EarthVisionGrid`); null nodes are dropped.
    #[staticmethod]
    fn load_earthvision_grid(py: Python<'_>, path: &str) -> PyResult<PointSet> {
        py.detach(|| RsPointSet::load_earthvision_grid(path))
            .map(PointSet::owned)
            .map_err(to_pyerr)
    }

    /// Build an in-memory `PointSet` from equal-length `x`/`y`/`z` lists.
    #[staticmethod]
    fn from_xyz(x: Vec<f64>, y: Vec<f64>, z: Vec<f64>) -> PyResult<PointSet> {
        if x.len() != y.len() || x.len() != z.len() {
            return Err(PyValueError::new_err(
                "from_xyz: x, y, z must have equal length",
            ));
        }
        let coords: Vec<[f64; 3]> = (0..x.len()).map(|i| [x[i], y[i], z[i]]).collect();
        Ok(PointSet::owned(RsPointSet::from_coords(coords)))
    }

    fn __len__(&self, py: Python<'_>) -> PyResult<usize> {
        self.with(py, |p| Ok(p.len()))
    }

    /// NaN-skipping statistics over the points' **z** coordinate (horizon
    /// depth/elevation range).
    fn z_stats(&self, py: Python<'_>) -> PyResult<Stats> {
        self.with(py, |p| Ok(Stats::new(p.z_stats())))
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

    /// Infer a regular grid geometry from the points. Raises `ValueError` when
    /// the point cloud is not grid-like within `tolerance`.
    ///
    /// `edge` controls `geometry.edge`: `"occupied"` (default), `"convex_hull"`,
    /// or `"full_rect"`.
    #[pyo3(signature = (tolerance = 1e-3, edge = "occupied"))]
    fn infer_geometry(&self, py: Python<'_>, tolerance: f64, edge: &str) -> PyResult<GridGeometry> {
        let edge = parse_geometry_edge(edge)?;
        self.with(py, |p| {
            let (geom, edge_polygon) = p
                .infer_geometry_with_edge(tolerance, edge)
                .map_err(to_pyerr)?;
            Ok(GridGeometry::with_edge(geom, edge_polygon))
        })
    }

    /// Grid the points' Z values onto `geom` using `method` (`"nearest"`,
    /// `"idw"`, or `"min_curvature"`), returning a new `Surface`.
    #[pyo3(signature = (geom, method = "idw"))]
    fn to_surface(&self, py: Python<'_>, geom: &GridGeometry, method: &str) -> PyResult<Surface> {
        let gm = parse_grid_method(method)?;
        let g = geom.inner.clone();
        // Gridding (esp. min-curvature) is compute-heavy pure Rust — release the GIL.
        self.with(py, |p| {
            py.detach(|| p.to_surface(g, gm))
                .map(Surface::wrap)
                .map_err(to_pyerr)
        })
    }

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        self.with(py, |p| Ok(format!("PointSet(len={})", p.len())))
    }
}

fn parse_geometry_edge(s: &str) -> PyResult<GeometryEdge> {
    match s
        .trim()
        .to_ascii_lowercase()
        .replace([' ', '-'], "_")
        .as_str()
    {
        "occupied" => Ok(GeometryEdge::Occupied),
        "convex_hull" | "convexhull" | "hull" => Ok(GeometryEdge::ConvexHull),
        "full_rect" | "fullrect" | "rect" | "rectangle" => Ok(GeometryEdge::FullRect),
        other => Err(PyValueError::new_err(format!(
            "unknown geometry edge '{other}' (expected 'occupied', 'convex_hull', or 'full_rect')"
        ))),
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
    /// Build an in-memory `PolygonSet` from `rings` — each ring a list of
    /// `[x, y, z]` (z optional/ignored) or `[x, y]` vertices. Rings with fewer
    /// than three vertices are dropped.
    #[staticmethod]
    fn from_rings(rings: Vec<Vec<Vec<f64>>>) -> PyResult<PolygonSet> {
        let mut out: Vec<Vec<[f64; 3]>> = Vec::with_capacity(rings.len());
        for ring in rings {
            let mut verts: Vec<[f64; 3]> = Vec::with_capacity(ring.len());
            for v in ring {
                if v.len() < 2 {
                    return Err(PyValueError::new_err(
                        "from_rings: each vertex needs at least [x, y]",
                    ));
                }
                verts.push([v[0], v[1], v.get(2).copied().unwrap_or(0.0)]);
            }
            out.push(verts);
        }
        Ok(PolygonSet::owned(RsPolygonSet::from_rings(out)))
    }

    /// Load polygons from a GeoJSON file.
    #[staticmethod]
    fn load_geojson(py: Python<'_>, path: &str) -> PyResult<PolygonSet> {
        py.detach(|| RsPolygonSet::load_geojson(path))
            .map(PolygonSet::owned)
            .map_err(to_pyerr)
    }

    /// Load polygons from an IRAP/RMS plain `X Y Z` file (`999.0` separators).
    #[staticmethod]
    fn load_irap_polygons(py: Python<'_>, path: &str) -> PyResult<PolygonSet> {
        py.detach(|| RsPolygonSet::load_irap_polygons(path))
            .map(PolygonSet::owned)
            .map_err(to_pyerr)
    }

    /// Load polygons from an ESRI shapefile (pass the `.shp` path).
    #[staticmethod]
    fn load_shapefile(py: Python<'_>, path: &str) -> PyResult<PolygonSet> {
        py.detach(|| RsPolygonSet::load_shapefile(path))
            .map(PolygonSet::owned)
            .map_err(to_pyerr)
    }

    /// Load polygons from a CPS-3 lines file (`.CPS3lines`) — polyline blocks
    /// each introduced by a `->` marker (structure outlines, faults, edges).
    #[staticmethod]
    fn load_cps3_lines(py: Python<'_>, path: &str) -> PyResult<PolygonSet> {
        py.detach(|| RsPolygonSet::load_cps3_lines(path))
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

    /// Exterior ring vertices of each polygon as `[x, y, z]` (z = 0); the
    /// outline geometry, not just area/bbox.
    fn rings(&self, py: Python<'_>) -> PyResult<Vec<Vec<[f64; 3]>>> {
        self.with(py, |p| Ok(p.rings()))
    }

    /// A copy of `surface` with every node outside all polygons masked to `NaN`.
    fn clip(&self, py: Python<'_>, surface: &Surface) -> PyResult<Surface> {
        let clipped = self.with(py, |p| surface.with(py, |s| p.clip(s)))?;
        Ok(Surface::wrap(clipped))
    }
}
