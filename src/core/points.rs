//! `PointSet` — scattered 3-D points (N×3 coords) with named `f64` attribute
//! columns, a spatial index for nearest-neighbour queries, and gridding onto a
//! `Surface` (`to_surface`). `NaN` = undefined. Imports from `foundation`,
//! `io`, and (for gridding) `core::surface`.
//!
//! Gridding is **delegated to the shared petekTools kernels** (`grid` cold,
//! `grid_min_curvature_seeded` warm). petekTools' `Lattice` is field-for-field
//! identical to our `GridGeometry`, so the seam is a 1:1 map (`to_lattice`); the
//! kernels themselves were lifted from petekIO 0.2.0 and are held at parity.

use crate::core::shell::{fit_grid_from_coords, fit_grid_from_indexed};
use crate::core::{PolygonSet, StructuredMeshSurface, Surface};
use crate::foundation::{
    BBox, GeoError, GridGeometry, HasHistory, OperationHistory, Point3, Result, Stats,
};
use crate::io::PointData;
use indexmap::IndexMap;
use ndarray::Array2;
use petektools::{grid as pt_grid, grid_min_curvature_seeded, GridMethod as PtGridMethod};
use rstar::primitives::GeomWithData;
use rstar::RTree;
use std::collections::HashMap;
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
    /// Outline of the occupied lattice nodes — the true data footprint, which
    /// follows interior holes and a non-rectangular outer boundary.
    Occupied,
    /// Convex hull of the input point cloud.
    ConvexHull,
    /// The full rectangular lattice footprint. Cheap and always four corners, but
    /// it claims the whole bounding lattice even where no node carries data.
    FullRect,
}

impl GridMethod {
    /// Map onto petekTools' identically-named method enum at the kernel seam.
    pub(crate) fn to_petektools(self) -> PtGridMethod {
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
    /// caller also needs the modelling edge polygon — in particular
    /// [`GeometryEdge::Occupied`] when the data footprint is not rectangular.
    pub fn infer_geometry(&self, tolerance: f64) -> Result<GridGeometry> {
        self.infer_geometry_with_edge(tolerance, GeometryEdge::FullRect)
            .map(|(geom, _edge)| geom)
    }

    /// Infer a regular grid geometry and an edge polygon. This is intentionally
    /// strict: genuinely scattered points, ambiguous axes, duplicate lattice
    /// nodes, or coordinates that miss the inferred lattice by more than
    /// `tolerance` return `GeoError::GeometryInference`. A curvilinear mesh — one
    /// carrying `column`/`row` whose nodes do not sit on any regular lattice — is
    /// likewise rejected; represent it with [`to_structured_surface`](Self::to_structured_surface).
    pub fn infer_geometry_with_edge(
        &self,
        tolerance: f64,
        edge: GeometryEdge,
    ) -> Result<(GridGeometry, PolygonSet)> {
        // The topology path resolves occupancy from `column`/`row`; the coordinate path
        // hands back the lattice indices it derived. Either way the occupied footprint is
        // already known — no point cloud needs triangulating to recover it.
        let (geom, occupancy) =
            match infer_grid_geometry_from_index_attrs(&self.coords, &self.attrs, tolerance)? {
                Some(geom) => (geom, None),
                None => {
                    let (geom, occupancy) =
                        fit_grid_from_coords(&self.coords, tolerance).map_err(|err| {
                            add_xy_only_inference_hint(err, &self.coords, &self.attrs, tolerance)
                        })?;
                    (geom, Some(occupancy))
                }
            };
        let edge_polygon = match edge {
            GeometryEdge::Occupied => match &occupancy {
                Some(occupancy) => occupied_edge_from_lattice_indices(&geom, occupancy)?,
                None => topology_occupied_edge_from_points(&self.coords, &self.attrs)?,
            },
            GeometryEdge::FullRect => PolygonSet::from_grid_geometry(&geom),
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

    /// Promote topology-bearing points to a structured mesh surface. This keeps
    /// the exported logical `(column, row)` topology and the actual per-node XY
    /// coordinates, instead of forcing the nodes onto a single affine
    /// [`GridGeometry`].
    ///
    /// Requires `column`/`row` attributes. Use [`to_surface`](Self::to_surface)
    /// when the desired result is a regular grid on an explicit model geometry.
    pub fn to_structured_surface(
        &self,
        tolerance: f64,
        edge: GeometryEdge,
    ) -> Result<StructuredMeshSurface> {
        let indexed = topology_indexed_points(&self.coords, &self.attrs)?;
        if indexed.len() < 4 {
            return Err(GeoError::GeometryInference(
                "structured surface conversion requires at least four indexed points".into(),
            ));
        }

        let min_col = indexed.iter().map(|p| p.col).min().unwrap();
        let max_col = indexed.iter().map(|p| p.col).max().unwrap();
        let min_row = indexed.iter().map(|p| p.row).min().unwrap();
        let max_row = indexed.iter().map(|p| p.row).max().unwrap();
        if max_col <= min_col || max_row <= min_row {
            return Err(GeoError::GeometryInference(
                "column/row attributes do not span a two-dimensional structured surface".into(),
            ));
        }

        let ncol = (max_col - min_col + 1) as usize;
        let nrow = (max_row - min_row + 1) as usize;
        let mut x = Array2::from_elem((ncol, nrow), f64::NAN);
        let mut y = Array2::from_elem((ncol, nrow), f64::NAN);
        let mut values = Array2::from_elem((ncol, nrow), f64::NAN);
        let mut occupied = vec![false; ncol * nrow];

        for p in &indexed {
            let i = (p.col - min_col) as usize;
            let j = (p.row - min_row) as usize;
            let slot = i + j * ncol;
            if occupied[slot] {
                return Err(GeoError::GeometryInference(format!(
                    "multiple points map to structured node column={}, row={}",
                    p.col, p.row
                )));
            }
            occupied[slot] = true;
            x[[i, j]] = p.x;
            y[[i, j]] = p.y;
            values[[i, j]] = p.z;
        }

        let nominal_geometry =
            infer_grid_geometry_from_index_attrs(&self.coords, &self.attrs, tolerance)
                .ok()
                .flatten();
        let edge_polygon = structured_edge(&x, &y, Some(&values), nominal_geometry.as_ref(), edge)?;
        let mut out = StructuredMeshSurface::new(x, y, values, nominal_geometry, edge_polygon)?;
        let mut history = self.history.clone();
        history.push(format!("points.to_structured_surface(edge={edge:?})"));
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

#[derive(Debug, Clone, Copy)]
struct IndexedPoint {
    col: isize,
    row: isize,
    x: f64,
    y: f64,
    z: f64,
}

fn topology_indexed_points(
    coords: &[[f64; 3]],
    attrs: &IndexMap<String, Vec<f64>>,
) -> Result<Vec<IndexedPoint>> {
    let columns = find_attr(attrs, &["column", "col"]).ok_or_else(|| {
        GeoError::GeometryInference(
            "structured surface conversion requires column/row topology attributes".into(),
        )
    })?;
    let rows = find_attr(attrs, &["row"]).ok_or_else(|| {
        GeoError::GeometryInference(
            "structured surface conversion requires column/row topology attributes".into(),
        )
    })?;
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
        indexed.push(IndexedPoint {
            col,
            row,
            x: c[0],
            y: c[1],
            z: c[2],
        });
    }
    Ok(indexed)
}

fn topology_occupied_edge_from_points(
    coords: &[[f64; 3]],
    attrs: &IndexMap<String, Vec<f64>>,
) -> Result<PolygonSet> {
    let indexed = topology_indexed_points(coords, attrs)?;
    if indexed.len() < 4 {
        return Err(GeoError::GeometryInference(
            "topology-aware occupied edge requires at least four indexed points".into(),
        ));
    }

    let min_col = indexed.iter().map(|p| p.col).min().unwrap();
    let max_col = indexed.iter().map(|p| p.col).max().unwrap();
    let min_row = indexed.iter().map(|p| p.row).min().unwrap();
    let max_row = indexed.iter().map(|p| p.row).max().unwrap();
    if max_col <= min_col || max_row <= min_row {
        return Err(GeoError::GeometryInference(
            "column/row attributes do not span a two-dimensional occupied edge".into(),
        ));
    }

    let ncol = (max_col - min_col + 1) as usize;
    let nrow = (max_row - min_row + 1) as usize;
    let mut x = Array2::from_elem((ncol, nrow), f64::NAN);
    let mut y = Array2::from_elem((ncol, nrow), f64::NAN);
    let mut occupied = vec![false; ncol * nrow];

    for p in indexed {
        let i = (p.col - min_col) as usize;
        let j = (p.row - min_row) as usize;
        let slot = i + j * ncol;
        if occupied[slot] {
            return Err(GeoError::GeometryInference(format!(
                "multiple points map to topology node column={}, row={}",
                p.col, p.row
            )));
        }
        occupied[slot] = true;
        x[[i, j]] = p.x;
        y[[i, j]] = p.y;
    }

    occupied_edge_from_node_arrays(&x, &y, None)
        .or_else(|_| perimeter_edge_from_node_arrays(&x, &y))
        .or_else(|_| convex_hull_from_node_arrays(&x, &y))
}

fn structured_edge(
    x: &Array2<f64>,
    y: &Array2<f64>,
    values: Option<&Array2<f64>>,
    nominal_geometry: Option<&GridGeometry>,
    edge: GeometryEdge,
) -> Result<PolygonSet> {
    match edge {
        GeometryEdge::ConvexHull => convex_hull_from_node_arrays(x, y),
        GeometryEdge::FullRect => nominal_geometry
            .map(PolygonSet::from_grid_geometry)
            .ok_or_else(|| {
                GeoError::GeometryInference(
                    "full_rect edge requires a nominal regular geometry; this mesh is \
                     curvilinear — use the occupied edge"
                        .into(),
                )
            }),
        GeometryEdge::Occupied => occupied_edge_from_node_arrays(x, y, values)
            .or_else(|_| perimeter_edge_from_node_arrays(x, y))
            .or_else(|_| convex_hull_from_node_arrays(x, y)),
    }
}

fn convex_hull_from_node_arrays(x: &Array2<f64>, y: &Array2<f64>) -> Result<PolygonSet> {
    let pts = x
        .iter()
        .zip(y.iter())
        .filter(|(x, y)| x.is_finite() && y.is_finite())
        .map(|(x, y)| [*x, *y])
        .collect();
    PolygonSet::convex_hull_xy(pts).ok_or_else(|| {
        GeoError::GeometryInference(
            "structured surface edge requires at least three non-collinear nodes".into(),
        )
    })
}

fn perimeter_edge_from_node_arrays(x: &Array2<f64>, y: &Array2<f64>) -> Result<PolygonSet> {
    let (ncol, nrow) = x.dim();
    if ncol < 2 || nrow < 2 {
        return Err(GeoError::GeometryInference(
            "structured surface perimeter edge requires at least 2x2 nodes".into(),
        ));
    }
    let mut ring = Vec::new();
    for i in 0..ncol {
        push_finite_node(&mut ring, x, y, i, 0)?;
    }
    for j in 1..nrow {
        push_finite_node(&mut ring, x, y, ncol - 1, j)?;
    }
    for i in (0..ncol.saturating_sub(1)).rev() {
        push_finite_node(&mut ring, x, y, i, nrow - 1)?;
    }
    for j in (1..nrow.saturating_sub(1)).rev() {
        push_finite_node(&mut ring, x, y, 0, j)?;
    }
    if ring.len() < 3 {
        return Err(GeoError::GeometryInference(
            "structured surface perimeter edge has fewer than three finite nodes".into(),
        ));
    }
    Ok(PolygonSet::from_rings(vec![ring]))
}

fn occupied_edge_from_node_arrays(
    x: &Array2<f64>,
    y: &Array2<f64>,
    values: Option<&Array2<f64>>,
) -> Result<PolygonSet> {
    let (ncol, nrow) = x.dim();
    if ncol < 2 || nrow < 2 {
        return Err(GeoError::GeometryInference(
            "occupied edge requires at least 2x2 nodes".into(),
        ));
    }
    if let Some(z) = values {
        if z.dim() != (ncol, nrow) {
            return Err(GeoError::GeometryInference(
                "occupied edge value array does not match node geometry".into(),
            ));
        }
    }

    let mut node_present = vec![false; ncol * nrow];
    for j in 0..nrow {
        for i in 0..ncol {
            let xy_present = x[[i, j]].is_finite() && y[[i, j]].is_finite();
            let value_present = values.map(|z| z[[i, j]].is_finite()).unwrap_or(true);
            node_present[i + j * ncol] = xy_present && value_present;
        }
    }

    let cell_ncol = ncol - 1;
    let cell_nrow = nrow - 1;
    let mut cell_present = vec![false; cell_ncol * cell_nrow];
    for j in 0..cell_nrow {
        for i in 0..cell_ncol {
            cell_present[i + j * cell_ncol] = node_present[i + j * ncol]
                && node_present[i + 1 + j * ncol]
                && node_present[i + 1 + (j + 1) * ncol]
                && node_present[i + (j + 1) * ncol];
        }
    }

    let has_cell = |cells: &[bool], i: isize, j: isize| -> bool {
        if i < 0 || j < 0 || i >= cell_ncol as isize || j >= cell_nrow as isize {
            return false;
        }
        cells[i as usize + j as usize * cell_ncol]
    };

    let mut next: HashMap<(usize, usize), Vec<(usize, usize)>> = HashMap::new();
    let mut edge_count = 0usize;
    for j in 0..cell_nrow {
        for i in 0..cell_ncol {
            if !cell_present[i + j * cell_ncol] {
                continue;
            }
            if !has_cell(&cell_present, i as isize, j as isize - 1) {
                add_boundary_edge(&mut next, &mut edge_count, (i, j), (i + 1, j));
            }
            if !has_cell(&cell_present, i as isize + 1, j as isize) {
                add_boundary_edge(&mut next, &mut edge_count, (i + 1, j), (i + 1, j + 1));
            }
            if !has_cell(&cell_present, i as isize, j as isize + 1) {
                add_boundary_edge(&mut next, &mut edge_count, (i + 1, j + 1), (i, j + 1));
            }
            if !has_cell(&cell_present, i as isize - 1, j as isize) {
                add_boundary_edge(&mut next, &mut edge_count, (i, j + 1), (i, j));
            }
        }
    }

    if edge_count == 0 {
        return Err(GeoError::GeometryInference(
            "occupied edge has no complete occupied cells".into(),
        ));
    }

    let mut rings = Vec::new();
    while let Some((start, mut current)) = pop_any_boundary_edge(&mut next) {
        let mut ring_nodes = vec![start];
        while current != start {
            ring_nodes.push(current);
            if ring_nodes.len() > edge_count + 1 {
                return Err(GeoError::GeometryInference(
                    "occupied edge tracing did not close".into(),
                ));
            }
            current = pop_boundary_edge(&mut next, current).ok_or_else(|| {
                GeoError::GeometryInference("occupied edge boundary is not closed".into())
            })?;
        }
        if ring_nodes.len() >= 3 {
            rings.push(
                ring_nodes
                    .into_iter()
                    .map(|(i, j)| [x[[i, j]], y[[i, j]], 0.0])
                    .collect::<Vec<_>>(),
            );
        }
    }

    if rings.is_empty() {
        return Err(GeoError::GeometryInference(
            "occupied edge has fewer than three boundary vertices".into(),
        ));
    }
    Ok(PolygonSet::from_rings(rings))
}

fn add_boundary_edge(
    next: &mut HashMap<(usize, usize), Vec<(usize, usize)>>,
    edge_count: &mut usize,
    from: (usize, usize),
    to: (usize, usize),
) {
    next.entry(from).or_default().push(to);
    *edge_count += 1;
}

fn pop_any_boundary_edge(
    next: &mut HashMap<(usize, usize), Vec<(usize, usize)>>,
) -> Option<((usize, usize), (usize, usize))> {
    let from = *next
        .iter()
        .find(|(_, outs)| !outs.is_empty())
        .map(|(k, _)| k)?;
    let to = pop_boundary_edge(next, from)?;
    Some((from, to))
}

fn pop_boundary_edge(
    next: &mut HashMap<(usize, usize), Vec<(usize, usize)>>,
    from: (usize, usize),
) -> Option<(usize, usize)> {
    let out = {
        let outs = next.get_mut(&from)?;
        outs.pop()
    };
    if next.get(&from).is_some_and(Vec::is_empty) {
        next.remove(&from);
    }
    out
}

fn push_finite_node(
    ring: &mut Vec<[f64; 3]>,
    x: &Array2<f64>,
    y: &Array2<f64>,
    i: usize,
    j: usize,
) -> Result<()> {
    let xi = x[[i, j]];
    let yi = y[[i, j]];
    if !xi.is_finite() || !yi.is_finite() {
        return Err(GeoError::GeometryInference(
            "structured surface perimeter edge has missing boundary nodes".into(),
        ));
    }
    ring.push([xi, yi, 0.0]);
    Ok(())
}

impl HasHistory for PointSet {
    fn operation_history(&self) -> &OperationHistory {
        &self.history
    }

    fn operation_history_mut(&mut self) -> &mut OperationHistory {
        &mut self.history
    }
}

/// Infer a regular lattice from bare coordinates, returning it alongside the lattice
/// index of every finite point — the occupancy the edge builders need.
/// The occupied-node footprint of a lattice, from the node indices inference already
/// resolved. Feeds the same edge builder the topology path uses, so a point set without
/// `column`/`row` gets an identical footprint without triangulating the cloud.
fn occupied_edge_from_lattice_indices(
    geom: &GridGeometry,
    occupancy: &[(usize, usize)],
) -> Result<PolygonSet> {
    let mut x = Array2::from_elem((geom.ncol, geom.nrow), f64::NAN);
    let mut y = Array2::from_elem((geom.ncol, geom.nrow), f64::NAN);
    for &(i, j) in occupancy {
        let (nx, ny) = geom.node_xy(i, j);
        x[[i, j]] = nx;
        y[[i, j]] = ny;
    }
    occupied_edge_from_node_arrays(&x, &y, None)
        .or_else(|_| perimeter_edge_from_node_arrays(&x, &y))
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
    // The fit itself is single-homed in `core::shell::fit` (shared with
    // `StructuredShell::infer_grid` / `MeshShell::infer_grid`).
    fit_grid_from_indexed(&indexed, tolerance).map(Some)
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
        "{message}; duplicate XY nodes were found and no column/row topology attributes are \
         available. Petrel surface point exports can lose grid topology in plain IRAP-points \
         form; an EarthVisionGrid export (or a point file carrying column/row) restores it. Note \
         that topology alone does not make a mesh regular — if the recovered mesh is curvilinear \
         it has no GridGeometry at all, and to_structured_surface is the exact representation"
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
    fn infer_geometry_rejects_curvilinear_mesh_carrying_column_row() {
        // A curvilinear mesh: regular column/row topology, but the node spacing swells
        // across the grid so no single (xinc, yinc, rotation) lattice fits it. The
        // median-delta estimator will still produce a plausible GridGeometry; inference
        // must refuse it rather than hand back a lattice the nodes do not sit on.
        let (ncol, nrow) = (12usize, 12usize);
        let mut coords = Vec::new();
        let mut columns = Vec::new();
        let mut rows = Vec::new();
        for j in 0..nrow {
            for i in 0..ncol {
                let swell = 1.0 + 0.06 * i as f64;
                let x = 1000.0 + 50.0 * i as f64 * swell;
                let y = 2000.0 + 50.0 * j as f64 * (1.0 + 0.04 * j as f64);
                coords.push([x, y, 10.0]);
                columns.push((i + 1) as f64);
                rows.push((j + 1) as f64);
            }
        }
        let mut attrs = IndexMap::new();
        attrs.insert("column".to_string(), columns);
        attrs.insert("row".to_string(), rows);
        let p = PointSet::from_parts(coords, attrs);

        let msg = match p.infer_geometry_with_edge(1e-3, GeometryEdge::FullRect) {
            Ok(_) => panic!("a curvilinear mesh has no regular GridGeometry"),
            Err(err) => err.to_string(),
        };
        assert!(msg.contains("curvilinear"), "unexpected message: {msg}");
        assert!(
            msg.contains("to_structured_surface"),
            "the error must point at the exact representation: {msg}"
        );

        let tri = p
            .to_tri_surface(None, None)
            .expect("a rejected regular lattice must remain representable as a TIN");
        assert!(!tri.triangles().is_empty());
        assert!(tri.points().len() <= p.len());

        // ...and the structured path, which stores explicit per-node XY, still works:
        // a curvilinear mesh simply has no nominal regular geometry.
        let mesh = p
            .to_structured_surface(1e-3, GeometryEdge::Occupied)
            .expect("structured surface represents a curvilinear mesh exactly");
        assert_eq!(mesh.ncol(), ncol);
        assert_eq!(mesh.nrow(), nrow);
        assert!(
            mesh.nominal_geometry().is_none(),
            "a curvilinear mesh must not advertise a regular nominal geometry"
        );
    }

    #[test]
    fn occupied_edge_follows_topology_cells() {
        let mut coords = Vec::new();
        let mut columns = Vec::new();
        let mut rows = Vec::new();
        for j in 0..4 {
            for i in 0..4 {
                if i <= 1 || j <= 1 {
                    coords.push([i as f64, j as f64, 100.0 + i as f64 + j as f64]);
                    columns.push((i + 1) as f64);
                    rows.push((j + 1) as f64);
                }
            }
        }

        let mut attrs = IndexMap::new();
        attrs.insert("column".to_string(), columns);
        attrs.insert("row".to_string(), rows);
        let p = PointSet::from_parts(coords, attrs);

        let (_, occupied) = p
            .infer_geometry_with_edge(1e-6, GeometryEdge::Occupied)
            .unwrap();
        let (_, hull) = p
            .infer_geometry_with_edge(1e-6, GeometryEdge::ConvexHull)
            .unwrap();
        let (_, full_rect) = p
            .infer_geometry_with_edge(1e-6, GeometryEdge::FullRect)
            .unwrap();

        // The L-shaped footprint: `occupied` tracks it, the rectangle over-claims.
        approx::assert_relative_eq!(occupied.area(), 5.0, epsilon = 1e-12);
        approx::assert_relative_eq!(full_rect.area(), 9.0, epsilon = 1e-12);
        assert!(hull.area() > occupied.area());
        assert!(full_rect.area() > hull.area());
    }

    #[test]
    fn occupied_and_full_rect_agree_on_a_fully_populated_lattice() {
        // When every node of the bounding lattice carries data there is no footprint to
        // track, and the two edges must coincide. They diverge only where nodes are
        // missing — which is precisely the case `full_rect` cannot express.
        let source = GridGeometry {
            xori: 100.0,
            yori: 200.0,
            xinc: 10.0,
            yinc: 25.0,
            ncol: 5,
            nrow: 4,
            rotation_deg: 20.0,
            yflip: false,
        };
        let mut coords = Vec::new();
        for j in 0..source.nrow {
            for i in 0..source.ncol {
                let (x, y) = source.node_xy(i, j);
                coords.push([x, y, 1.0]);
            }
        }
        let p = PointSet::from_coords(coords);

        let (geom, occupied) = p
            .infer_geometry_with_edge(1e-9, GeometryEdge::Occupied)
            .unwrap();
        let (_, full_rect) = p
            .infer_geometry_with_edge(1e-9, GeometryEdge::FullRect)
            .unwrap();

        assert_eq!(geom.ncol, source.ncol);
        assert_eq!(geom.nrow, source.nrow);
        let expected = (4.0 * 10.0) * (3.0 * 25.0);
        approx::assert_relative_eq!(full_rect.area(), expected, epsilon = 1e-9);
        approx::assert_relative_eq!(occupied.area(), expected, epsilon = 1e-9);
    }

    #[test]
    fn occupied_edge_without_topology_matches_the_topology_footprint() {
        // Same L-shaped lattice as `occupied_edge_follows_topology_cells`, but with no
        // column/row attributes. The coordinate path derives each point's lattice index
        // anyway, so both paths must agree on the footprint — the occupancy is the same
        // fact, however it arrives. (Before unification this path triangulated the cloud
        // and answered 5.5.)
        let mut coords = Vec::new();
        for j in 0..4 {
            for i in 0..4 {
                if i <= 1 || j <= 1 {
                    coords.push([i as f64, j as f64, 100.0 + i as f64 + j as f64]);
                }
            }
        }
        let p = PointSet::from_coords(coords);

        let (_, occupied) = p
            .infer_geometry_with_edge(1e-6, GeometryEdge::Occupied)
            .unwrap();
        let (_, full_rect) = p
            .infer_geometry_with_edge(1e-6, GeometryEdge::FullRect)
            .unwrap();

        approx::assert_relative_eq!(occupied.area(), 5.0, epsilon = 1e-12);
        approx::assert_relative_eq!(full_rect.area(), 9.0, epsilon = 1e-12);
    }

    #[test]
    fn to_structured_surface_preserves_locally_shifted_nodes() {
        let coords = vec![
            [0.0, 0.0, 100.0],
            [10.0, 0.0, 101.0],
            [0.0, 10.0, 102.0],
            [12.0, 10.0, 103.0],
        ];
        let mut attrs = IndexMap::new();
        attrs.insert("column".to_string(), vec![1.0, 2.0, 1.0, 2.0]);
        attrs.insert("row".to_string(), vec![1.0, 1.0, 2.0, 2.0]);
        let p = PointSet::from_parts(coords, attrs);

        let s = p
            .to_structured_surface(1e-3, GeometryEdge::Occupied)
            .unwrap();

        assert_eq!(s.kind(), "structured_mesh");
        assert_eq!(s.ncol(), 2);
        assert_eq!(s.nrow(), 2);
        assert_eq!(s.node_xy(1, 1).unwrap(), (12.0, 10.0));
        assert_eq!(s.z(1, 1).unwrap(), 103.0);
        assert_eq!(s.stats().count, 4);
        assert!(s.edge().area() > 0.0);
        assert!(s
            .history()
            .iter()
            .any(|h| h == "points.to_structured_surface(edge=Occupied)"));
    }

    #[test]
    fn structured_surface_occupied_edge_tracks_concave_cells() {
        let mut coords = Vec::new();
        let mut columns = Vec::new();
        let mut rows = Vec::new();
        for j in 0..4 {
            for i in 0..4 {
                if i <= 1 || j <= 1 {
                    coords.push([i as f64, j as f64, 100.0 + i as f64 + j as f64]);
                    columns.push((i + 1) as f64);
                    rows.push((j + 1) as f64);
                }
            }
        }

        let mut attrs = IndexMap::new();
        attrs.insert("column".to_string(), columns);
        attrs.insert("row".to_string(), rows);
        let p = PointSet::from_parts(coords, attrs);

        let s = p
            .to_structured_surface(1e-6, GeometryEdge::Occupied)
            .unwrap();

        approx::assert_relative_eq!(s.edge().area(), 5.0, epsilon = 1e-12);
    }

    #[test]
    fn to_structured_surface_requires_topology() {
        let p = PointSet::from_coords(vec![
            [0.0, 0.0, 100.0],
            [10.0, 0.0, 101.0],
            [0.0, 10.0, 102.0],
            [10.0, 10.0, 103.0],
        ]);

        let err = match p.to_structured_surface(1e-3, GeometryEdge::Occupied) {
            Ok(_) => panic!("expected missing topology error"),
            Err(err) => err,
        };
        assert!(format!("{err}").contains("requires column/row topology"));
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
