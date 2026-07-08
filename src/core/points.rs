//! `PointSet` — scattered 3-D points (N×3 coords) with named `f64` attribute
//! columns, a spatial index for nearest-neighbour queries, and gridding onto a
//! `Surface` (`to_surface`). `NaN` = undefined. Imports from `foundation`,
//! `io`, and (for gridding) `core::surface`.
//!
//! Gridding is **delegated to the shared petekTools kernels** (`grid` cold,
//! `grid_min_curvature_seeded` warm). petekTools' `Lattice` is field-for-field
//! identical to our `GridGeometry`, so the seam is a 1:1 map (`to_lattice`); the
//! kernels themselves were lifted from petekIO 0.2.0 and are held at parity.

use crate::core::{PolygonSet, Surface};
use crate::foundation::{
    BBox, GeoError, GridGeometry, HasHistory, OperationHistory, Point3, Result, Stats,
};
use crate::io::PointData;
use indexmap::IndexMap;
use petektools::{grid as pt_grid, grid_min_curvature_seeded, GridMethod as PtGridMethod};
use rstar::primitives::GeomWithData;
use rstar::RTree;
use std::path::Path;

/// A gridding method for [`PointSet::to_surface`] — see
/// `dev-docs/designs/gridding-method.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GridMethod {
    /// Value of the single areally-closest sample (blocky, exact at data).
    Nearest,
    /// Inverse-distance weighting, `wᵢ = 1/dᵢ²` (power p=2), exact at d=0.
    InverseDistance,
    /// Briggs minimum-curvature (biharmonic SOR relaxation, data-anchored).
    MinimumCurvature,
}

/// Edge polygon to attach when inferring a grid geometry from points.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GeometryEdge {
    /// Rectangular footprint spanning the occupied lattice index extents.
    Occupied,
    /// Convex hull of the input point cloud.
    ConvexHull,
    /// Full rectangular lattice footprint.
    FullRect,
}

impl GridMethod {
    /// Map onto petekTools' identically-named method enum at the kernel seam.
    fn to_petektools(self) -> PtGridMethod {
        match self {
            GridMethod::Nearest => PtGridMethod::Nearest,
            GridMethod::InverseDistance => PtGridMethod::InverseDistance,
            GridMethod::MinimumCurvature => PtGridMethod::MinimumCurvature,
        }
    }
}

/// An areal R*-tree entry: a 2-D `[x, y]` position carrying the point's index.
pub(crate) type AerialEntry = GeomWithData<[f64; 2], usize>;

/// Scattered points with attribute columns. Coordinates are stored as `[x, y,
/// z]`; each attribute is a `f64` column aligned 1:1 with `coords`.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct PointSet {
    pub(crate) coords: Vec<[f64; 3]>,
    pub(crate) attrs: IndexMap<String, Vec<f64>>,
    #[serde(default)]
    history: OperationHistory,
}

impl PointSet {
    /// Build a `PointSet` from raw coordinates and attribute columns. Each
    /// attribute column must match `coords.len()` (callers within the crate
    /// guarantee this).
    pub(crate) fn from_parts(coords: Vec<[f64; 3]>, attrs: IndexMap<String, Vec<f64>>) -> PointSet {
        PointSet {
            coords,
            attrs,
            history: OperationHistory::new(),
        }
    }

    pub(crate) fn from_point_data(data: PointData) -> PointSet {
        let (coords, attrs) = data.into_parts();
        PointSet::from_parts(coords, attrs)
    }

    /// Build an in-memory `PointSet` from `[x, y, z]` coordinates (no named
    /// attributes) — construct scattered points directly, without a file.
    pub fn from_coords(coords: Vec<[f64; 3]>) -> PointSet {
        let mut out = PointSet::from_parts(coords, IndexMap::new());
        out.history = OperationHistory::from_entry("points.from_coords");
        out
    }

    /// Read a headered CSV, taking X/Y/Z from the named columns. Every other
    /// column whose values all parse as `f64` becomes an attribute; columns
    /// with any non-numeric cell are skipped. Rows with a non-numeric X/Y/Z are
    /// an error (readers validate on load).
    pub fn load_csv(path: impl AsRef<Path>, x: &str, y: &str, z: &str) -> Result<PointSet> {
        let mut out =
            PointSet::from_point_data(crate::io::csv_points::load(path.as_ref(), x, y, z)?);
        out.history = OperationHistory::from_entry(format!(
            "points.load_csv(path={})",
            path.as_ref().display()
        ));
        Ok(out)
    }

    /// Load point features from a GeoJSON file. Each feature's numeric
    /// `properties{}` become attribute columns (the union of all features'
    /// numeric property names, NaN-filling features that lack one); string and
    /// other non-numeric properties are ignored.
    pub fn load_geojson(path: impl AsRef<Path>) -> Result<PointSet> {
        let mut out =
            PointSet::from_point_data(crate::io::vector::load_point_set_geojson(path.as_ref())?);
        out.history = OperationHistory::from_entry(format!(
            "points.load_geojson(path={})",
            path.as_ref().display()
        ));
        Ok(out)
    }

    /// Load scattered points from an IRAP/RMS plain `X Y Z` file. No named
    /// attributes (the format carries none). Format-sniffed: a foreign header
    /// (EarthVision grid / CPS-3 / LAS) is rejected with `GeoError::Format`.
    pub fn load_irap_points(path: impl AsRef<Path>) -> Result<PointSet> {
        let mut out = PointSet::from_point_data(crate::io::xyz::load_points(path.as_ref())?);
        out.history = OperationHistory::from_entry(format!(
            "points.load_irap_points(path={})",
            path.as_ref().display()
        ));
        Ok(out)
    }

    /// Load plain IRAP/RMS `X Y Z` points and transfer Petrel `column`/`row`
    /// topology from a matching EarthVision grid export. This is intentionally
    /// a project-loader helper: the IRAP file owns the returned coordinates,
    /// while `topology_path` contributes only grid indices for exact geometry
    /// inference.
    pub fn load_irap_points_with_topology(
        path: impl AsRef<Path>,
        topology_path: impl AsRef<Path>,
    ) -> Result<PointSet> {
        let points = crate::io::xyz::load_points(path.as_ref())?;
        let topology = crate::io::earthvision::load_earthvision_grid(topology_path.as_ref())?;
        let mut out =
            PointSet::from_point_data(points.with_topology_from_ordered_subset(&topology, 1e-3)?);
        out.history = OperationHistory::from_entry(format!(
            "points.load_irap_points_with_topology(path={}, topology_path={})",
            path.as_ref().display(),
            topology_path.as_ref().display()
        ));
        Ok(out)
    }

    /// Load scattered points from an EarthVision grid ASCII file
    /// (`.EarthVisionGrid`) — `x y z` nodes with a directive header; null nodes
    /// dropped (see [`crate::io::earthvision`]). Petrel `column`/`row` fields,
    /// when present, are preserved as attributes so geometry inference can use
    /// the exported grid topology instead of guessing from XY alone.
    pub fn load_earthvision_grid(path: impl AsRef<Path>) -> Result<PointSet> {
        let mut out = PointSet::from_point_data(crate::io::earthvision::load_earthvision_grid(
            path.as_ref(),
        )?);
        out.history = OperationHistory::from_entry(format!(
            "points.load_earthvision_grid(path={})",
            path.as_ref().display()
        ));
        Ok(out)
    }

    /// Number of points.
    pub fn len(&self) -> usize {
        self.coords.len()
    }

    /// Whether the set is empty.
    pub fn is_empty(&self) -> bool {
        self.coords.is_empty()
    }

    /// The raw `[x, y, z]` coordinates of every point, in load order (`NaN` =
    /// undefined, carried through as stored). The read side of
    /// [`from_coords`](Self::from_coords): a downstream consumer that grids the
    /// scatter itself (rather than through [`to_surface`](Self::to_surface)) reads
    /// the points here.
    pub fn coords(&self) -> &[[f64; 3]] {
        &self.coords
    }

    /// A new `PointSet` keeping only points for which `pred` is true. Attribute
    /// columns are carried over for the retained rows.
    pub fn filter(&self, pred: impl Fn(Point3) -> bool) -> PointSet {
        let keep: Vec<usize> = (0..self.coords.len())
            .filter(|&i| {
                let c = self.coords[i];
                pred(Point3::new(c[0], c[1], c[2]))
            })
            .collect();
        let coords = keep.iter().map(|&i| self.coords[i]).collect();
        let attrs = self
            .attrs
            .iter()
            .map(|(name, col)| (name.clone(), keep.iter().map(|&i| col[i]).collect()))
            .collect();
        let mut out = PointSet::from_parts(coords, attrs);
        out.history = self.history_with("points.filter");
        out
    }

    /// A named attribute column, if present.
    pub fn attr(&self, name: &str) -> Option<&[f64]> {
        self.attrs.get(name).map(Vec::as_slice)
    }

    /// Set (or replace) a named attribute column. The column must be aligned
    /// 1:1 with this point set's rows.
    pub fn set_attr(&mut self, name: &str, values: Vec<f64>) -> Result<()> {
        if values.len() != self.coords.len() {
            return Err(GeoError::Parse(format!(
                "point attribute '{name}' has {} rows, expected {}",
                values.len(),
                self.coords.len()
            )));
        }
        self.attrs.insert(name.to_string(), values);
        self.record_history(format!("points.set_attr(name={name})"));
        Ok(())
    }

    /// The names of all attribute columns, in insertion order.
    pub fn attr_names(&self) -> Vec<&str> {
        self.attrs.keys().map(String::as_str).collect()
    }

    /// Human-readable operation history for this point set.
    pub fn history(&self) -> &[String] {
        self.history.entries()
    }

    pub(crate) fn history_with(&self, entry: impl Into<String>) -> OperationHistory {
        self.history.with_entry(entry)
    }

    pub(crate) fn record_history(&mut self, entry: impl Into<String>) {
        self.history.push(entry.into());
    }

    /// NaN-skipping statistics over a named attribute column, or `None` if the
    /// attribute is absent.
    pub fn stats(&self, attr: &str) -> Option<Stats> {
        self.attrs.get(attr).map(|col| Stats::of(col))
    }

    /// NaN-skipping statistics over the points' **z** coordinate — the horizon
    /// depth/elevation range of a scattered set loaded as `X Y Z` (which stores
    /// z as a coordinate, not a named attribute).
    pub fn z_stats(&self) -> Stats {
        let z: Vec<f64> = self.coords.iter().map(|c| c[2]).collect();
        Stats::of(&z)
    }

    /// Axis-aligned bounding box of the points' XY. Empty set → a degenerate
    /// box of `NaN`s.
    pub fn bbox(&self) -> BBox {
        let mut b = BBox {
            xmin: f64::INFINITY,
            ymin: f64::INFINITY,
            xmax: f64::NEG_INFINITY,
            ymax: f64::NEG_INFINITY,
        };
        for c in &self.coords {
            b.xmin = b.xmin.min(c[0]);
            b.xmax = b.xmax.max(c[0]);
            b.ymin = b.ymin.min(c[1]);
            b.ymax = b.ymax.max(c[1]);
        }
        if self.coords.is_empty() {
            b = BBox {
                xmin: f64::NAN,
                ymin: f64::NAN,
                xmax: f64::NAN,
                ymax: f64::NAN,
            };
        }
        b
    }

    /// Index of the areally-nearest point to `(x, y)` (Euclidean in XY; Z is
    /// ignored). `None` for an empty set.
    pub fn nearest(&self, x: f64, y: f64) -> Option<usize> {
        if self.coords.is_empty() {
            return None;
        }
        let tree = self.rtree_xy();
        tree.nearest_neighbor([x, y]).map(|e| e.data)
    }

    /// Infer a regular grid geometry from point coordinates. The returned
    /// geometry spans the occupied lattice extents; use
    /// [`infer_geometry_with_edge`](Self::infer_geometry_with_edge) when the
    /// caller also needs the modelling edge polygon.
    pub fn infer_geometry(&self, tolerance: f64) -> Result<GridGeometry> {
        self.infer_geometry_with_edge(tolerance, GeometryEdge::Occupied)
            .map(|(geom, _edge)| geom)
    }

    /// Infer a regular grid geometry and an edge polygon. This is intentionally
    /// strict: genuinely scattered points, ambiguous axes, duplicate lattice
    /// nodes, or coordinates that miss the inferred lattice by more than
    /// `tolerance` return `GeoError::GeometryInference`.
    pub fn infer_geometry_with_edge(
        &self,
        tolerance: f64,
        edge: GeometryEdge,
    ) -> Result<(GridGeometry, PolygonSet)> {
        let geom = match infer_grid_geometry_from_index_attrs(&self.coords, &self.attrs, tolerance)?
        {
            Some(geom) => geom,
            None => infer_grid_geometry_from_coords(&self.coords, tolerance).map_err(|err| {
                add_xy_only_inference_hint(err, &self.coords, &self.attrs, tolerance)
            })?,
        };
        let edge_polygon = match edge {
            GeometryEdge::Occupied | GeometryEdge::FullRect => {
                PolygonSet::from_grid_geometry(&geom)
            }
            GeometryEdge::ConvexHull => {
                let pts = self
                    .coords
                    .iter()
                    .filter(|c| c[0].is_finite() && c[1].is_finite())
                    .map(|c| [c[0], c[1]])
                    .collect();
                PolygonSet::convex_hull_xy(pts).ok_or_else(|| {
                    GeoError::GeometryInference(
                        "convex hull edge requires at least three non-collinear points".into(),
                    )
                })?
            }
        };
        Ok((geom, edge_polygon))
    }

    /// Grid the points' Z values onto `geom` using `method`, returning a new
    /// `Surface`. See `dev-docs/designs/gridding-method.md`.
    pub fn to_surface(&self, geom: GridGeometry, method: GridMethod) -> Result<Surface> {
        let values = pt_grid(&self.coords, &geom.to_lattice(), method.to_petektools())?;
        let mut out = Surface::new(geom, values)?;
        let mut history = self.history.clone();
        history.push(format!("points.to_surface(method={method:?})"));
        out.set_history(history);
        Ok(out)
    }

    /// Warm-started minimum-curvature re-grid onto `prior`'s lattice, relaxing
    /// from `prior`'s values instead of a cold IDW seed. For an incremental
    /// re-grid (control points nudged, a point added) this converges much faster
    /// than [`to_surface`](Self::to_surface) with `MinimumCurvature` while giving
    /// the same converged field. Honours the points as hard constraints, as the
    /// cold path does.
    pub fn regrid_min_curvature(&self, prior: &Surface) -> Result<Surface> {
        let values = grid_min_curvature_seeded(
            &self.coords,
            &prior.geom.to_lattice(),
            Some(prior.values()),
        )?;
        let mut out = Surface::new(prior.geom.clone(), values)?;
        let mut history = self.history.clone();
        history.extend_prefixed("prior", prior.operation_history());
        history.push("points.regrid_min_curvature(prior)".to_string());
        out.set_history(history);
        Ok(out)
    }

    /// Build an areal R*-tree over the points' XY, payloaded with their index.
    pub(crate) fn rtree_xy(&self) -> RTree<AerialEntry> {
        let entries: Vec<AerialEntry> = self
            .coords
            .iter()
            .enumerate()
            .map(|(i, c)| GeomWithData::new([c[0], c[1]], i))
            .collect();
        RTree::bulk_load(entries)
    }
}

impl HasHistory for PointSet {
    fn operation_history(&self) -> &OperationHistory {
        &self.history
    }

    fn operation_history_mut(&mut self) -> &mut OperationHistory {
        &mut self.history
    }
}

fn infer_grid_geometry_from_coords(coords: &[[f64; 3]], tolerance: f64) -> Result<GridGeometry> {
    if !tolerance.is_finite() || tolerance <= 0.0 {
        return Err(GeoError::GeometryInference(
            "tolerance must be a finite positive number".into(),
        ));
    }

    let pts: Vec<[f64; 2]> = coords
        .iter()
        .filter(|c| c[0].is_finite() && c[1].is_finite())
        .map(|c| [c[0], c[1]])
        .collect();
    if pts.len() < 4 {
        return Err(GeoError::GeometryInference(
            "at least four finite points are required".into(),
        ));
    }

    let vectors = neighbour_vectors(&pts, tolerance);
    if vectors.len() < 2 {
        return Err(GeoError::GeometryInference(
            "not enough neighbouring points to detect grid axes".into(),
        ));
    }

    let (e1, e2, xinc, yinc) = infer_axes_and_spacing(&vectors, tolerance)?;
    let anchor = pts[0];
    let mut uv: Vec<(f64, f64)> = Vec::with_capacity(pts.len());
    for p in &pts {
        let dx = p[0] - anchor[0];
        let dy = p[1] - anchor[1];
        uv.push((dx * e1[0] + dy * e1[1], dx * e2[0] + dy * e2[1]));
    }

    let min_u = uv.iter().map(|p| p.0).fold(f64::INFINITY, f64::min);
    let min_v = uv.iter().map(|p| p.1).fold(f64::INFINITY, f64::min);
    let mut ij: Vec<(isize, isize)> = Vec::with_capacity(uv.len());
    let mut max_i = 0isize;
    let mut max_j = 0isize;
    let mut max_residual = 0.0_f64;

    for (u, v) in uv {
        let fi = (u - min_u) / xinc;
        let fj = (v - min_v) / yinc;
        let i = fi.round() as isize;
        let j = fj.round() as isize;
        if i < 0 || j < 0 {
            return Err(GeoError::GeometryInference(
                "inferred negative lattice index; grid origin is ambiguous".into(),
            ));
        }
        let du = (fi - i as f64).abs() * xinc;
        let dv = (fj - j as f64).abs() * yinc;
        let residual = du.hypot(dv);
        max_residual = max_residual.max(residual);
        if residual > tolerance {
            return Err(GeoError::GeometryInference(format!(
                "point misses inferred lattice by {residual:.6}, above tolerance {tolerance:.6}"
            )));
        }
        max_i = max_i.max(i);
        max_j = max_j.max(j);
        ij.push((i, j));
    }

    ij.sort_unstable();
    if ij.windows(2).any(|w| w[0] == w[1]) {
        return Err(GeoError::GeometryInference(
            "multiple points map to the same inferred grid node".into(),
        ));
    }
    if max_i < 1 || max_j < 1 {
        return Err(GeoError::GeometryInference(
            "detected points do not span a two-dimensional grid".into(),
        ));
    }

    let xori = anchor[0] + min_u * e1[0] + min_v * e2[0];
    let yori = anchor[1] + min_u * e1[1] + min_v * e2[1];
    let rotation_deg = e1[1].atan2(e1[0]).to_degrees();

    let geom = GridGeometry {
        xori,
        yori,
        xinc,
        yinc,
        ncol: (max_i + 1) as usize,
        nrow: (max_j + 1) as usize,
        rotation_deg,
        yflip: false,
    };

    if max_residual > tolerance {
        return Err(GeoError::GeometryInference(format!(
            "maximum lattice residual {max_residual:.6} exceeds tolerance {tolerance:.6}"
        )));
    }
    Ok(geom)
}

fn infer_grid_geometry_from_index_attrs(
    coords: &[[f64; 3]],
    attrs: &IndexMap<String, Vec<f64>>,
    tolerance: f64,
) -> Result<Option<GridGeometry>> {
    if !tolerance.is_finite() || tolerance <= 0.0 {
        return Err(GeoError::GeometryInference(
            "tolerance must be a finite positive number".into(),
        ));
    }
    let Some(columns) = find_attr(attrs, &["column", "col"]) else {
        return Ok(None);
    };
    let Some(rows) = find_attr(attrs, &["row"]) else {
        return Ok(None);
    };
    if columns.len() != coords.len() || rows.len() != coords.len() {
        return Err(GeoError::GeometryInference(
            "column/row attributes must match point count".into(),
        ));
    }

    let mut indexed = Vec::new();
    for (idx, c) in coords.iter().enumerate() {
        if !c[0].is_finite() || !c[1].is_finite() {
            continue;
        }
        let Some(col) = integer_attr(columns[idx], "column")? else {
            continue;
        };
        let Some(row) = integer_attr(rows[idx], "row")? else {
            continue;
        };
        indexed.push((col, row, c[0], c[1]));
    }
    if indexed.len() < 4 {
        return Err(GeoError::GeometryInference(
            "column/row geometry inference requires at least four indexed points".into(),
        ));
    }

    let min_col = indexed.iter().map(|p| p.0).min().unwrap();
    let max_col = indexed.iter().map(|p| p.0).max().unwrap();
    let min_row = indexed.iter().map(|p| p.1).min().unwrap();
    let max_row = indexed.iter().map(|p| p.1).max().unwrap();
    if max_col <= min_col || max_row <= min_row {
        return Err(GeoError::GeometryInference(
            "column/row attributes do not span a two-dimensional grid".into(),
        ));
    }

    let mut by_index = std::collections::BTreeMap::new();
    for (col, row, x, y) in &indexed {
        by_index.entry((*col, *row)).or_insert((*x, *y));
    }

    let mut i_dx = Vec::new();
    let mut i_dy = Vec::new();
    let mut j_dx = Vec::new();
    let mut j_dy = Vec::new();
    for ((col, row), (x, y)) in &by_index {
        if let Some((nx, ny)) = by_index.get(&(*col + 1, *row)) {
            let dx = nx - x;
            let dy = ny - y;
            if dx.hypot(dy) > tolerance {
                i_dx.push(dx);
                i_dy.push(dy);
            }
        }
        if let Some((nx, ny)) = by_index.get(&(*col, *row + 1)) {
            let dx = nx - x;
            let dy = ny - y;
            if dx.hypot(dy) > tolerance {
                j_dx.push(dx);
                j_dy.push(dy);
            }
        }
    }

    let (Some(m_i_dx), Some(m_i_dy), Some(m_j_dx), Some(m_j_dy)) = (
        median_unsorted(&i_dx),
        median_unsorted(&i_dy),
        median_unsorted(&j_dx),
        median_unsorted(&j_dy),
    ) else {
        return Err(GeoError::GeometryInference(
            "column/row attributes are present, but adjacent indexed nodes are too sparse to infer spacing".into(),
        ));
    };

    let e1 = unit([m_i_dx, m_i_dy])?;
    let xinc = m_i_dx.hypot(m_i_dy);
    let perp = [-e1[1], e1[0]];
    let row_projection = m_j_dx * perp[0] + m_j_dy * perp[1];
    if row_projection.abs() <= tolerance {
        return Err(GeoError::GeometryInference(
            "column/row attributes imply a degenerate row spacing".into(),
        ));
    }
    let yinc = row_projection.abs();
    let yflip = row_projection < 0.0;
    let ysign = if yflip { -1.0 } else { 1.0 };

    let mut origins_x = Vec::with_capacity(indexed.len());
    let mut origins_y = Vec::with_capacity(indexed.len());
    for (col, row, x, y) in indexed {
        let i = (col - min_col) as f64;
        let j = (row - min_row) as f64;
        origins_x.push(x - i * xinc * e1[0] - j * yinc * ysign * perp[0]);
        origins_y.push(y - i * xinc * e1[1] - j * yinc * ysign * perp[1]);
    }
    let xori = median_unsorted(&origins_x)
        .ok_or_else(|| GeoError::GeometryInference("could not infer indexed grid origin".into()))?;
    let yori = median_unsorted(&origins_y)
        .ok_or_else(|| GeoError::GeometryInference("could not infer indexed grid origin".into()))?;

    Ok(Some(GridGeometry {
        xori,
        yori,
        xinc,
        yinc,
        ncol: (max_col - min_col + 1) as usize,
        nrow: (max_row - min_row + 1) as usize,
        rotation_deg: e1[1].atan2(e1[0]).to_degrees(),
        yflip,
    }))
}

fn find_attr<'a>(attrs: &'a IndexMap<String, Vec<f64>>, names: &[&str]) -> Option<&'a [f64]> {
    attrs.iter().find_map(|(key, values)| {
        let normalized = crate::io::normalize_attr_name(key);
        names
            .iter()
            .any(|name| normalized == *name)
            .then_some(values.as_slice())
    })
}

fn integer_attr(value: f64, name: &str) -> Result<Option<isize>> {
    if value.is_nan() {
        return Ok(None);
    }
    if !value.is_finite() {
        return Err(GeoError::GeometryInference(format!(
            "{name} attribute contains a non-finite value"
        )));
    }
    let rounded = value.round();
    if (value - rounded).abs() > 1e-6 {
        return Err(GeoError::GeometryInference(format!(
            "{name} attribute contains non-integer grid indices"
        )));
    }
    Ok(Some(rounded as isize))
}

fn median_unsorted(values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.total_cmp(b));
    median(&sorted)
}

fn add_xy_only_inference_hint(
    err: GeoError,
    coords: &[[f64; 3]],
    attrs: &IndexMap<String, Vec<f64>>,
    tolerance: f64,
) -> GeoError {
    let GeoError::GeometryInference(message) = err else {
        return err;
    };
    let has_index_attrs =
        find_attr(attrs, &["column", "col"]).is_some() || find_attr(attrs, &["row"]).is_some();
    if has_index_attrs || !has_duplicate_xy(coords, tolerance) {
        return GeoError::GeometryInference(message);
    }
    GeoError::GeometryInference(format!(
        "{message}; duplicate XY nodes were found and no column/row topology attributes are available. Petrel surface point exports can lose grid topology in plain IRAP-points form; use an EarthVisionGrid export or a point file carrying column/row when exact geometry inference is required"
    ))
}

fn has_duplicate_xy(coords: &[[f64; 3]], tolerance: f64) -> bool {
    let scale = 1.0 / tolerance.max(1e-9);
    let mut seen = std::collections::HashSet::new();
    for c in coords {
        if !c[0].is_finite() || !c[1].is_finite() {
            continue;
        }
        let key = ((c[0] * scale).round() as i64, (c[1] * scale).round() as i64);
        if !seen.insert(key) {
            return true;
        }
    }
    false
}

fn neighbour_vectors(pts: &[[f64; 2]], tolerance: f64) -> Vec<[f64; 2]> {
    let entries: Vec<AerialEntry> = pts
        .iter()
        .enumerate()
        .map(|(i, p)| GeomWithData::new(*p, i))
        .collect();
    let tree = RTree::bulk_load(entries);
    let stride = (pts.len() / 2000).max(1);
    let mut vectors = Vec::new();

    for (idx, p) in pts.iter().enumerate().step_by(stride) {
        for neighbour in tree.nearest_neighbor_iter(*p).take(13) {
            if neighbour.data == idx {
                continue;
            }
            let q = pts[neighbour.data];
            let dx = q[0] - p[0];
            let dy = q[1] - p[1];
            if dx.hypot(dy) > tolerance {
                vectors.push([dx, dy]);
            }
        }
    }
    vectors
}

fn infer_axes_and_spacing(
    vectors: &[[f64; 2]],
    tolerance: f64,
) -> Result<([f64; 2], [f64; 2], f64, f64)> {
    let mut by_len: Vec<[f64; 2]> = vectors.to_vec();
    by_len.sort_by(|a, b| a[0].hypot(a[1]).total_cmp(&b[0].hypot(b[1])));

    let first = *by_len
        .first()
        .ok_or_else(|| GeoError::GeometryInference("no neighbour vectors found".into()))?;
    let mut e1 = unit(first)?;
    if e1[0] < 0.0 || (e1[0].abs() <= f64::EPSILON && e1[1] < 0.0) {
        e1 = [-e1[0], -e1[1]];
    }
    let e2 = [-e1[1], e1[0]];

    let xinc = spacing_along(vectors, e1, tolerance)?;
    let yinc = spacing_along(vectors, e2, tolerance)?;
    Ok((e1, e2, xinc, yinc))
}

fn unit(v: [f64; 2]) -> Result<[f64; 2]> {
    let d = v[0].hypot(v[1]);
    if d == 0.0 || !d.is_finite() {
        return Err(GeoError::GeometryInference(
            "zero-length vector while detecting grid axes".into(),
        ));
    }
    Ok([v[0] / d, v[1] / d])
}

fn spacing_along(vectors: &[[f64; 2]], axis: [f64; 2], tolerance: f64) -> Result<f64> {
    let mut projected: Vec<f64> = vectors
        .iter()
        .filter_map(|v| {
            let along = (v[0] * axis[0] + v[1] * axis[1]).abs();
            let across = (v[0] * -axis[1] + v[1] * axis[0]).abs();
            (along > tolerance && across <= tolerance).then_some(along)
        })
        .collect();
    projected.sort_by(|a, b| a.total_cmp(b));
    let shortest = projected.first().copied().ok_or_else(|| {
        GeoError::GeometryInference("could not detect regular spacing on both grid axes".into())
    })?;
    let near_step: Vec<f64> = projected
        .into_iter()
        .filter(|v| *v <= shortest * 1.5 + tolerance)
        .collect();
    median(&near_step).ok_or_else(|| {
        GeoError::GeometryInference("could not detect regular spacing on both grid axes".into())
    })
}

fn median(values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let mid = values.len() / 2;
    if values.len().is_multiple_of(2) {
        Some((values[mid - 1] + values[mid]) * 0.5)
    } else {
        Some(values[mid])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pts() -> PointSet {
        let coords = vec![[0.0, 0.0, 1.0], [10.0, 0.0, 2.0], [0.0, 10.0, 3.0]];
        let mut attrs = IndexMap::new();
        attrs.insert("poro".to_string(), vec![0.1, 0.2, 0.3]);
        PointSet::from_parts(coords, attrs)
    }

    #[test]
    fn len_and_attr_and_stats() {
        let p = pts();
        assert_eq!(p.len(), 3);
        assert!(!p.is_empty());
        assert_eq!(p.attr("poro").unwrap(), &[0.1, 0.2, 0.3]);
        assert!(p.attr("missing").is_none());
        let s = p.stats("poro").unwrap();
        assert_eq!(s.count, 3);
        approx::assert_relative_eq!(s.mean, 0.2);
        assert!(p.stats("nope").is_none());
    }

    #[test]
    fn from_coords_and_z_stats() {
        let p = PointSet::from_coords(vec![
            [0.0, 0.0, -100.0],
            [1.0, 1.0, -120.0],
            [2.0, 2.0, -140.0],
        ]);
        assert_eq!(p.len(), 3);
        assert!(p.attr("z").is_none()); // z is a coordinate, not a named attr
        let z = p.z_stats();
        assert_eq!(z.count, 3);
        approx::assert_relative_eq!(z.mean, -120.0);
        approx::assert_relative_eq!(z.min, -140.0);
        approx::assert_relative_eq!(z.max, -100.0);
    }

    #[test]
    fn coords_round_trips_from_coords() {
        // The read side of `from_coords`: identical slice, load order preserved,
        // `NaN` carried through unchanged (undefined convention).
        let raw = vec![[0.0, 0.0, -100.0], [1.0, 1.0, f64::NAN], [2.0, 2.0, -140.0]];
        let p = PointSet::from_coords(raw.clone());
        let got = p.coords();
        assert_eq!(got.len(), raw.len());
        for (g, r) in got.iter().zip(&raw) {
            assert_eq!(g[0], r[0]);
            assert_eq!(g[1], r[1]);
            if r[2].is_nan() {
                assert!(g[2].is_nan(), "NaN must carry through");
            } else {
                assert_eq!(g[2], r[2]);
            }
        }
    }

    #[test]
    fn bbox_covers_points() {
        let b = pts().bbox();
        approx::assert_relative_eq!(b.xmin, 0.0);
        approx::assert_relative_eq!(b.xmax, 10.0);
        approx::assert_relative_eq!(b.ymin, 0.0);
        approx::assert_relative_eq!(b.ymax, 10.0);
    }

    #[test]
    fn nearest_matches_brute_force() {
        let p = pts();
        // brute-force nearest to a few query points
        let queries = [(1.0, 1.0), (9.0, 1.0), (1.0, 9.0), (5.0, 5.0)];
        for (qx, qy) in queries {
            let brute = (0..p.len())
                .min_by(|&a, &b| {
                    let da = (p.coords[a][0] - qx).powi(2) + (p.coords[a][1] - qy).powi(2);
                    let db = (p.coords[b][0] - qx).powi(2) + (p.coords[b][1] - qy).powi(2);
                    da.total_cmp(&db)
                })
                .unwrap();
            assert_eq!(p.nearest(qx, qy), Some(brute));
        }
    }

    #[test]
    fn filter_keeps_matching_rows_and_attrs() {
        let p = pts().filter(|pt| pt.x < 5.0);
        assert_eq!(p.len(), 2); // (0,0) and (0,10)
        assert_eq!(p.attr("poro").unwrap(), &[0.1, 0.3]);
    }

    #[test]
    fn empty_nearest_is_none() {
        let p = PointSet::from_parts(Vec::new(), IndexMap::new());
        assert!(p.is_empty());
        assert!(p.nearest(0.0, 0.0).is_none());
    }

    #[test]
    fn infer_geometry_recovers_rotated_lattice() {
        let source = GridGeometry {
            xori: 456_123.5,
            yori: 6_712_345.25,
            xinc: 37.0,
            yinc: 83.0,
            ncol: 5,
            nrow: 4,
            rotation_deg: 27.5,
            yflip: false,
        };
        let mut coords = Vec::new();
        for j in 0..source.nrow {
            for i in 0..source.ncol {
                let (x, y) = source.node_xy(i, j);
                coords.push([x, y, 1000.0 + i as f64 + j as f64]);
            }
        }

        let p = PointSet::from_coords(coords);
        let (geom, edge) = p
            .infer_geometry_with_edge(1e-6, GeometryEdge::FullRect)
            .unwrap();
        approx::assert_relative_eq!(geom.xori, source.xori, epsilon = 1e-6);
        approx::assert_relative_eq!(geom.yori, source.yori, epsilon = 1e-6);
        approx::assert_relative_eq!(geom.xinc, source.xinc, epsilon = 1e-9);
        approx::assert_relative_eq!(geom.yinc, source.yinc, epsilon = 1e-9);
        assert_eq!(geom.ncol, source.ncol);
        assert_eq!(geom.nrow, source.nrow);
        approx::assert_relative_eq!(geom.rotation_deg, source.rotation_deg, epsilon = 1e-9);
        approx::assert_relative_eq!(edge.area(), (4.0 * 37.0) * (3.0 * 83.0), epsilon = 1e-6);
    }

    #[test]
    fn infer_geometry_uses_explicit_column_row_topology() {
        let source = GridGeometry {
            xori: 1000.0,
            yori: 2000.0,
            xinc: 50.0,
            yinc: 25.0,
            ncol: 4,
            nrow: 3,
            rotation_deg: 35.0,
            yflip: false,
        };
        let mut coords = Vec::new();
        let mut columns = Vec::new();
        let mut rows = Vec::new();
        for j in 0..source.nrow {
            for i in 0..source.ncol {
                let (mut x, mut y) = source.node_xy(i, j);
                if i == 2 && j == 1 {
                    x += 3.0;
                    y -= 2.0;
                }
                coords.push([x, y, 1000.0 + i as f64 + j as f64]);
                columns.push((i + 1) as f64);
                rows.push((j + 1) as f64);
            }
        }
        // A repeated XY node can happen in Petrel point exports around collapsed
        // or clipped grid nodes. With column/row present, topology still wins.
        coords[2] = coords[1];

        let mut attrs = IndexMap::new();
        attrs.insert("column".to_string(), columns);
        attrs.insert("row".to_string(), rows);
        let p = PointSet::from_parts(coords, attrs);
        let (geom, edge) = p
            .infer_geometry_with_edge(1e-3, GeometryEdge::ConvexHull)
            .unwrap();

        assert_eq!(geom.ncol, source.ncol);
        assert_eq!(geom.nrow, source.nrow);
        approx::assert_relative_eq!(geom.xinc, source.xinc, epsilon = 1e-9);
        approx::assert_relative_eq!(geom.yinc, source.yinc, epsilon = 1e-9);
        approx::assert_relative_eq!(geom.rotation_deg, source.rotation_deg, epsilon = 1e-9);
        assert!(edge.area() > 0.0);
    }

    #[test]
    fn infer_geometry_errors_for_scattered_points() {
        let p = PointSet::from_coords(vec![
            [0.0, 0.0, 1.0],
            [11.0, 0.2, 2.0],
            [3.0, 8.7, 3.0],
            [19.0, 4.1, 4.0],
            [7.0, 17.3, 5.0],
        ]);
        assert!(matches!(
            p.infer_geometry(1e-3),
            Err(GeoError::GeometryInference(_))
        ));
    }

    fn grid5() -> crate::foundation::GridGeometry {
        crate::foundation::GridGeometry {
            xori: 0.0,
            yori: 0.0,
            xinc: 2.5,
            yinc: 2.5,
            ncol: 5,
            nrow: 5,
            rotation_deg: 0.0,
            yflip: false,
        }
    }

    #[test]
    fn warm_start_honours_constraints_and_converges() {
        let p = pts();
        let cold = p.to_surface(grid5(), GridMethod::MinimumCurvature).unwrap();
        let warm = p.regrid_min_curvature(&cold).unwrap();
        assert_eq!(warm.geom, cold.geom);
        // Hard constraints: each input point snaps to a node held at its z.
        // Points (0,0,1)→node[0,0], (10,0,2)→node[4,0], (0,10,3)→node[0,4].
        approx::assert_relative_eq!(warm.values()[[0, 0]], 1.0, epsilon = 1e-9);
        approx::assert_relative_eq!(warm.values()[[4, 0]], 2.0, epsilon = 1e-9);
        approx::assert_relative_eq!(warm.values()[[0, 4]], 3.0, epsilon = 1e-9);
        // A second warm pass is a near-fixed point (the field has converged).
        let warm2 = p.regrid_min_curvature(&warm).unwrap();
        for (a, b) in warm2.values().iter().zip(warm.values().iter()) {
            approx::assert_relative_eq!(a, b, epsilon = 1e-6);
        }
    }

    #[test]
    fn regrid_empty_errors() {
        let empty = PointSet::from_parts(Vec::new(), IndexMap::new());
        let prior = pts()
            .to_surface(grid5(), GridMethod::MinimumCurvature)
            .unwrap();
        assert!(empty.regrid_min_curvature(&prior).is_err());
    }
}
