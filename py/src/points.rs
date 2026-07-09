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
use crate::structured_surface::StructuredMeshSurface;
use crate::surface::Surface;
use crate::{parse_grid_method, to_pyerr};
use petekio::{
    GeometryEdge, PointSet as RsPointSet, PolygonSet as RsPolygonSet,
    TopologyReport as RsTopologyReport,
};
use pyo3::exceptions::{PyAttributeError, PyTypeError, PyValueError};
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

    fn owned_mut(&mut self, py: Python<'_>) -> PyResult<&mut RsPointSet> {
        if let PointBacking::InGeo { geo, name } = &self.backing {
            let cloned = {
                let g = geo.borrow(py);
                let p = g
                    .inner
                    .points(name)
                    .ok_or_else(|| PyValueError::new_err(format!("no point set '{name}'")))?;
                p.clone()
            };
            self.backing = PointBacking::Owned(Arc::new(cloned));
        }
        match &mut self.backing {
            PointBacking::Owned(a) => Ok(Arc::make_mut(a)),
            PointBacking::InGeo { .. } => unreachable!("just detached to Owned"),
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

    /// Human-readable operation history for this point set.
    fn history(&self, py: Python<'_>) -> PyResult<Vec<String>> {
        self.with(py, |p| Ok(p.history().to_vec()))
    }

    /// NaN-skipping statistics over the points' **z** coordinate (horizon
    /// depth/elevation range).
    fn z_stats(&self, py: Python<'_>) -> PyResult<Stats> {
        self.with(py, |p| Ok(Stats::new(p.z_stats())))
    }

    /// Point coordinates as `(x, y)` tuples in load order.
    fn xy(&self, py: Python<'_>) -> PyResult<Vec<(f64, f64)>> {
        self.with(py, |p| {
            Ok(p.coords().iter().map(|c| (c[0], c[1])).collect())
        })
    }

    /// Point coordinates as `(x, y, z)` tuples in load order.
    fn xyz(&self, py: Python<'_>) -> PyResult<Vec<(f64, f64, f64)>> {
        self.with(py, |p| {
            Ok(p.coords().iter().map(|c| (c[0], c[1], c[2])).collect())
        })
    }

    /// Names of all numeric attribute columns.
    fn attr_names(&self, py: Python<'_>) -> PyResult<Vec<String>> {
        self.with(py, |p| {
            Ok(p.attr_names().iter().map(|n| n.to_string()).collect())
        })
    }

    /// Set (or replace) attribute `name` from a same-length column, list, or scalar.
    fn set_attr(&mut self, py: Python<'_>, name: &str, values: &Bound<'_, PyAny>) -> PyResult<()> {
        if is_reserved_point_column(name) {
            return Err(PyAttributeError::new_err(format!(
                "cannot assign coordinate column '{name}'; create a named attribute instead"
            )));
        }
        let n = self.__len__(py)?;
        let col = extract_numeric_column(values, n, "PointSet.set_attr")?;
        self.owned_mut(py)?.set_attr(name, col).map_err(to_pyerr)
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

    /// Recover `column`/`row` topology from bare coordinates, without moving a point.
    ///
    /// Returns `(points, report)`. `points` carries the `column`/`row` attributes and
    /// is **`None` unless the detection verified** — every distinct node labelled, no
    /// index claimed twice, no coincident node pair with differing z. An unverified
    /// report means the surface is fault-cut: represent it as a triangulated network,
    /// not a structured mesh. `report.stalled_frontier` locates the fault.
    #[pyo3(signature = (nominal_cell = None))]
    fn detect_topology(
        &self,
        py: Python<'_>,
        nominal_cell: Option<f64>,
    ) -> PyResult<(Option<PointSet>, TopologyReport)> {
        self.with(py, |p| {
            // Neighbour search over the whole cloud is compute-heavy pure Rust.
            let (labelled, report) = py
                .detach(|| p.detect_topology(nominal_cell))
                .map_err(to_pyerr)?;
            Ok((
                labelled.map(PointSet::owned),
                TopologyReport { inner: report },
            ))
        })
    }

    /// Infer a regular grid geometry from the points. Raises `ValueError` when
    /// the point cloud is not grid-like within `tolerance`.
    ///
    /// `edge` controls `geometry.edge`: `"full_rect"` (default; the four corners of
    /// the bounding lattice), `"occupied"` (the outline of the nodes that carry
    /// data — use this when the footprint is not rectangular), or `"convex_hull"`.
    #[pyo3(signature = (tolerance = 1e-3, edge = "full_rect"))]
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

    /// Promote topology-bearing points (`column`/`row` attributes) to a
    /// structured mesh surface with explicit XY at every logical node.
    #[pyo3(signature = (tolerance = 1e-3, edge = "occupied"))]
    fn to_structured_surface(
        &self,
        py: Python<'_>,
        tolerance: f64,
        edge: &str,
    ) -> PyResult<StructuredMeshSurface> {
        let edge = parse_geometry_edge(edge)?;
        self.with(py, |p| {
            p.to_structured_surface(tolerance, edge)
                .map(StructuredMeshSurface::wrap)
                .map_err(to_pyerr)
        })
    }

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        self.with(py, |p| Ok(format!("PointSet(len={})", p.len())))
    }

    /// Coordinate/attribute column access: `points.z + 2`, `points.PHIE * points.NTG`.
    fn __getattr__(&self, py: Python<'_>, name: &str) -> PyResult<PointColumn> {
        self.with(py, |p| {
            if name == "x" || name == "y" || name == "z" {
                let idx = match name {
                    "x" => 0,
                    "y" => 1,
                    _ => 2,
                };
                return Ok(PointColumn::new(
                    name.to_string(),
                    p.coords().iter().map(|c| c[idx]).collect(),
                ));
            }
            if let Some(values) = p.attr(name).or_else(|| find_point_attr(p, name)) {
                return Ok(PointColumn::new(name.to_string(), values.to_vec()));
            }
            Err(PyAttributeError::new_err(format!(
                "'PointSet' object has no attribute or column '{name}'"
            )))
        })
    }

    /// `points.new_attr = points.z + points.y` assigns a numeric attribute column.
    fn __setattr__(
        &mut self,
        py: Python<'_>,
        name: &str,
        value: &Bound<'_, PyAny>,
    ) -> PyResult<()> {
        self.set_attr(py, name, value)
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
        // Removed: the triangulated point-cloud hull was slow (a full Delaunay over
        // every point) and produced a stair-stepped ring. 'occupied' now yields the
        // same footprint from the lattice occupancy inference already resolves.
        removed @ ("concave_hull" | "concavehull" | "concave" | "alpha" | "alpha_shape"
        | "outer" | "default" | "trimesh" | "tin" | "triangulated"
        | "triangulated_mesh") => Err(PyValueError::new_err(format!(
            "geometry edge '{removed}' has been removed; use 'occupied' for the data \
                 footprint, 'full_rect' for the bounding lattice rectangle, or 'convex_hull'"
        ))),
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

    fn owned_mut(&mut self, py: Python<'_>) -> PyResult<&mut RsPolygonSet> {
        if let PolyBacking::InGeo { geo, name } = &self.backing {
            let cloned = {
                let g = geo.borrow(py);
                let p = g
                    .inner
                    .polygons(name)
                    .ok_or_else(|| PyValueError::new_err(format!("no polygon set '{name}'")))?;
                p.clone()
            };
            self.backing = PolyBacking::Owned(Arc::new(cloned));
        }
        match &mut self.backing {
            PolyBacking::Owned(a) => Ok(Arc::make_mut(a)),
            PolyBacking::InGeo { .. } => unreachable!("just detached to Owned"),
        }
    }
}

fn is_reserved_point_column(name: &str) -> bool {
    name == "x" || name == "y" || name == "z"
}

fn normalized(name: &str) -> String {
    name.trim().to_ascii_lowercase().replace([' ', '-'], "_")
}

fn find_point_attr<'a>(p: &'a RsPointSet, name: &str) -> Option<&'a [f64]> {
    let target = normalized(name);
    p.attr_names()
        .into_iter()
        .find(|candidate| normalized(candidate) == target)
        .and_then(|candidate| p.attr(candidate))
}

fn find_polygon_attr<'a>(p: &'a RsPolygonSet, name: &str) -> Option<&'a [f64]> {
    let target = normalized(name);
    p.attr_names()
        .into_iter()
        .find(|candidate| normalized(candidate) == target)
        .and_then(|candidate| p.attr(candidate))
}

fn extract_numeric_column(
    obj: &Bound<'_, PyAny>,
    expected: usize,
    ctx: &str,
) -> PyResult<Vec<f64>> {
    if let Ok(col) = obj.extract::<PyRef<'_, PointColumn>>() {
        return same_len(col.values.clone(), expected, ctx);
    }
    if let Ok(col) = obj.extract::<PyRef<'_, PolygonColumn>>() {
        return same_len(col.values.clone(), expected, ctx);
    }
    if let Ok(v) = obj.extract::<Vec<f64>>() {
        return same_len(v, expected, ctx);
    }
    if let Ok(v) = obj.extract::<f64>() {
        return Ok(vec![v; expected]);
    }
    Err(PyTypeError::new_err(format!(
        "{ctx}: expected a numeric column, a same-length list[float], or a scalar"
    )))
}

fn same_len(values: Vec<f64>, expected: usize, ctx: &str) -> PyResult<Vec<f64>> {
    if values.len() == expected {
        Ok(values)
    } else {
        Err(PyValueError::new_err(format!(
            "{ctx}: column has {} rows, expected {expected}",
            values.len()
        )))
    }
}

fn apply_column_op(
    lhs: &[f64],
    rhs: &Bound<'_, PyAny>,
    op_name: &str,
    op: impl Fn(f64, f64) -> f64,
) -> PyResult<Vec<f64>> {
    if let Ok(k) = rhs.extract::<f64>() {
        return Ok(lhs.iter().map(|v| op(*v, k)).collect());
    }
    if let Ok(col) = rhs.extract::<PyRef<'_, PointColumn>>() {
        if col.values.len() != lhs.len() {
            return Err(PyValueError::new_err(format!(
                "{op_name}: columns have different lengths ({} vs {})",
                lhs.len(),
                col.values.len()
            )));
        }
        return Ok(lhs
            .iter()
            .zip(col.values.iter())
            .map(|(a, b)| op(*a, *b))
            .collect());
    }
    if let Ok(col) = rhs.extract::<PyRef<'_, PolygonColumn>>() {
        if col.values.len() != lhs.len() {
            return Err(PyValueError::new_err(format!(
                "{op_name}: columns have different lengths ({} vs {})",
                lhs.len(),
                col.values.len()
            )));
        }
        return Ok(lhs
            .iter()
            .zip(col.values.iter())
            .map(|(a, b)| op(*a, *b))
            .collect());
    }
    Err(PyTypeError::new_err(format!(
        "{op_name}: operands must be a scalar or a column from the same container"
    )))
}

#[pyclass(name = "PointColumn", skip_from_py_object)]
#[derive(Clone)]
pub struct PointColumn {
    name: String,
    values: Vec<f64>,
}

impl PointColumn {
    fn new(name: String, values: Vec<f64>) -> PointColumn {
        PointColumn { name, values }
    }
}

#[pymethods]
impl PointColumn {
    fn values(&self) -> Vec<f64> {
        self.values.clone()
    }

    fn stats(&self) -> Stats {
        Stats::new(petekio::Stats::of(&self.values))
    }

    fn __len__(&self) -> usize {
        self.values.len()
    }

    fn __add__(&self, rhs: &Bound<'_, PyAny>) -> PyResult<PointColumn> {
        Ok(PointColumn::new(
            self.name.clone(),
            apply_column_op(&self.values, rhs, "PointColumn.__add__", |a, b| a + b)?,
        ))
    }

    fn __sub__(&self, rhs: &Bound<'_, PyAny>) -> PyResult<PointColumn> {
        Ok(PointColumn::new(
            self.name.clone(),
            apply_column_op(&self.values, rhs, "PointColumn.__sub__", |a, b| a - b)?,
        ))
    }

    fn __mul__(&self, rhs: &Bound<'_, PyAny>) -> PyResult<PointColumn> {
        Ok(PointColumn::new(
            self.name.clone(),
            apply_column_op(&self.values, rhs, "PointColumn.__mul__", |a, b| a * b)?,
        ))
    }

    fn __truediv__(&self, rhs: &Bound<'_, PyAny>) -> PyResult<PointColumn> {
        Ok(PointColumn::new(
            self.name.clone(),
            apply_column_op(&self.values, rhs, "PointColumn.__truediv__", |a, b| a / b)?,
        ))
    }

    fn __radd__(&self, lhs: f64) -> PointColumn {
        PointColumn::new(
            self.name.clone(),
            self.values.iter().map(|v| lhs + *v).collect(),
        )
    }

    fn __rsub__(&self, lhs: f64) -> PointColumn {
        PointColumn::new(
            self.name.clone(),
            self.values.iter().map(|v| lhs - *v).collect(),
        )
    }

    fn __rmul__(&self, lhs: f64) -> PointColumn {
        PointColumn::new(
            self.name.clone(),
            self.values.iter().map(|v| lhs * *v).collect(),
        )
    }

    fn __repr__(&self) -> String {
        format!(
            "PointColumn(name='{}', len={})",
            self.name,
            self.values.len()
        )
    }
}

#[pyclass(name = "PolygonColumn", skip_from_py_object)]
#[derive(Clone)]
pub struct PolygonColumn {
    name: String,
    values: Vec<f64>,
    total: Option<f64>,
}

impl PolygonColumn {
    fn new(name: String, values: Vec<f64>) -> PolygonColumn {
        PolygonColumn {
            name,
            values,
            total: None,
        }
    }

    fn with_total(name: String, values: Vec<f64>, total: f64) -> PolygonColumn {
        PolygonColumn {
            name,
            values,
            total: Some(total),
        }
    }
}

#[pymethods]
impl PolygonColumn {
    fn values(&self) -> Vec<f64> {
        self.values.clone()
    }

    fn stats(&self) -> Stats {
        Stats::new(petekio::Stats::of(&self.values))
    }

    fn __len__(&self) -> usize {
        self.values.len()
    }

    /// Compatibility: `polygons.area()` returns total area.
    fn __call__(&self) -> PyResult<f64> {
        self.total.ok_or_else(|| {
            PyTypeError::new_err(format!(
                "PolygonColumn '{}' is not callable; use .values() or assign it as an attribute",
                self.name
            ))
        })
    }

    fn __add__(&self, rhs: &Bound<'_, PyAny>) -> PyResult<PolygonColumn> {
        Ok(PolygonColumn::new(
            self.name.clone(),
            apply_column_op(&self.values, rhs, "PolygonColumn.__add__", |a, b| a + b)?,
        ))
    }

    fn __sub__(&self, rhs: &Bound<'_, PyAny>) -> PyResult<PolygonColumn> {
        Ok(PolygonColumn::new(
            self.name.clone(),
            apply_column_op(&self.values, rhs, "PolygonColumn.__sub__", |a, b| a - b)?,
        ))
    }

    fn __mul__(&self, rhs: &Bound<'_, PyAny>) -> PyResult<PolygonColumn> {
        Ok(PolygonColumn::new(
            self.name.clone(),
            apply_column_op(&self.values, rhs, "PolygonColumn.__mul__", |a, b| a * b)?,
        ))
    }

    fn __truediv__(&self, rhs: &Bound<'_, PyAny>) -> PyResult<PolygonColumn> {
        Ok(PolygonColumn::new(
            self.name.clone(),
            apply_column_op(&self.values, rhs, "PolygonColumn.__truediv__", |a, b| a / b)?,
        ))
    }

    fn __radd__(&self, lhs: f64) -> PolygonColumn {
        PolygonColumn::new(
            self.name.clone(),
            self.values.iter().map(|v| lhs + *v).collect(),
        )
    }

    fn __rsub__(&self, lhs: f64) -> PolygonColumn {
        PolygonColumn::new(
            self.name.clone(),
            self.values.iter().map(|v| lhs - *v).collect(),
        )
    }

    fn __rmul__(&self, lhs: f64) -> PolygonColumn {
        PolygonColumn::new(
            self.name.clone(),
            self.values.iter().map(|v| lhs * *v).collect(),
        )
    }

    fn __repr__(&self) -> String {
        format!(
            "PolygonColumn(name='{}', len={})",
            self.name,
            self.values.len()
        )
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

    fn __len__(&self, py: Python<'_>) -> PyResult<usize> {
        self.with(py, |p| Ok(p.len()))
    }

    /// Human-readable operation history for this polygon set.
    fn history(&self, py: Python<'_>) -> PyResult<Vec<String>> {
        self.with(py, |p| Ok(p.history().to_vec()))
    }

    /// Per-polygon unsigned-area column. Callable for compatibility:
    /// `polygons.area()` returns the total area.
    #[getter]
    fn area(&self, py: Python<'_>) -> PyResult<PolygonColumn> {
        self.with(py, |p| {
            Ok(PolygonColumn::with_total(
                "area".to_string(),
                p.area_values(),
                p.area(),
            ))
        })
    }

    /// Total unsigned area of all polygons.
    fn total_area(&self, py: Python<'_>) -> PyResult<f64> {
        self.with(py, |p| Ok(p.area()))
    }

    /// A named attribute column as a `list[float]`, or `None` if absent.
    fn attr(&self, py: Python<'_>, name: &str) -> PyResult<Option<Vec<f64>>> {
        self.with(py, |p| Ok(p.attr(name).map(|c| c.to_vec())))
    }

    /// Names of all numeric attribute columns.
    fn attr_names(&self, py: Python<'_>) -> PyResult<Vec<String>> {
        self.with(py, |p| {
            Ok(p.attr_names().iter().map(|n| n.to_string()).collect())
        })
    }

    /// Set (or replace) attribute `name` from a same-length column, list, or scalar.
    fn set_attr(&mut self, py: Python<'_>, name: &str, values: &Bound<'_, PyAny>) -> PyResult<()> {
        if name == "area" {
            return Err(PyAttributeError::new_err(
                "cannot assign derived polygon column 'area'; create a named attribute instead",
            ));
        }
        let n = self.__len__(py)?;
        let col = extract_numeric_column(values, n, "PolygonSet.set_attr")?;
        self.owned_mut(py)?.set_attr(name, col).map_err(to_pyerr)
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

    /// Attribute column access: `polygons.ntg * polygons.area`.
    fn __getattr__(&self, py: Python<'_>, name: &str) -> PyResult<PolygonColumn> {
        self.with(py, |p| {
            if let Some(values) = p.attr(name).or_else(|| find_polygon_attr(p, name)) {
                return Ok(PolygonColumn::new(name.to_string(), values.to_vec()));
            }
            Err(PyAttributeError::new_err(format!(
                "'PolygonSet' object has no attribute or column '{name}'"
            )))
        })
    }

    /// `polygons.net_area = polygons.area * polygons.ntg` assigns a numeric attribute column.
    fn __setattr__(
        &mut self,
        py: Python<'_>,
        name: &str,
        value: &Bound<'_, PyAny>,
    ) -> PyResult<()> {
        self.set_attr(py, name, value)
    }
}

/// What `PointSet.detect_topology(...)` learned. `verified` is the gate: labels are
/// returned only when it is true.
#[pyclass(name = "TopologyReport")]
pub struct TopologyReport {
    pub(crate) inner: RsTopologyReport,
}

#[pymethods]
impl TopologyReport {
    /// Every distinct node labelled, no index claimed twice, no unresolvable coincidence.
    #[getter]
    fn verified(&self) -> bool {
        self.inner.verified()
    }

    /// Detected cell size, from the modal nearest-neighbour step.
    #[getter]
    fn detected_cell(&self) -> f64 {
        self.inner.detected_cell
    }

    /// Detected grid azimuth in degrees, modulo 90.
    #[getter]
    fn detected_azimuth_deg(&self) -> f64 {
        self.inner.detected_azimuth_deg
    }

    /// Distinct nodes considered, after dropping exactly-coincident duplicates.
    #[getter]
    fn distinct_nodes(&self) -> usize {
        self.inner.distinct_nodes
    }

    /// Nodes the walk reached and labelled.
    #[getter]
    fn assigned(&self) -> usize {
        self.inner.assigned
    }

    /// Times two points claimed the same `(column, row)`.
    #[getter]
    fn conflicts(&self) -> usize {
        self.inner.conflicts
    }

    /// Coincident points dropped: same XY *and* same z, so harmless.
    #[getter]
    fn coincident_dropped(&self) -> usize {
        self.inner.coincident_dropped
    }

    /// Coincident points with differing z: two nodes at one place, unresolvable.
    #[getter]
    fn coincident_ambiguous(&self) -> usize {
        self.inner.coincident_ambiguous
    }

    /// Adjacencies the walk could not resolve — the fault traces.
    #[getter]
    fn stalled_frontier(&self) -> usize {
        self.inner.stalled_frontier
    }

    fn __repr__(&self) -> String {
        let r = &self.inner;
        format!(
            "TopologyReport(verified={}, assigned={}/{}, cell={:.3}, azimuth={:.3}, \
             conflicts={}, stalled_frontier={}, coincident_dropped={}, coincident_ambiguous={})",
            r.verified(),
            r.assigned,
            r.distinct_nodes,
            r.detected_cell,
            r.detected_azimuth_deg,
            r.conflicts,
            r.stalled_frontier,
            r.coincident_dropped,
            r.coincident_ambiguous,
        )
    }
}
