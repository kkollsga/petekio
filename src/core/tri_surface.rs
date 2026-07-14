//! `TriSurface` — the level-3 surface: an [`Arc<MeshShell>`] (geometry) plus a
//! primary per-node value lane (z) and named attribute lanes. The fallback
//! surface for points whose `(column, row)` topology cannot be verified.
//!
//! When [`PointSet::detect_topology`](crate::PointSet::detect_topology) stalls, the
//! surface is fault-cut and no structured mesh describes it. Triangulate the points
//! directly instead. Two things make that safe:
//!
//! * **A maximum link length**, expressed in *cells*, removes the long spanning
//!   slivers a Delaunay hull throws across a concave boundary. The triangulation runs
//!   in the **normalized grid frame** — each axis divided by its own step — so an
//!   anisotropic cell becomes a unit square and one scalar bound serves both axes. In
//!   world units no such scalar exists: past an aspect ratio of √2 the cell diagonal
//!   already exceeds two short-axis steps, and the admissible band is empty.
//! * **The walk's stalled frontier as fault constraints.** Link length alone cannot
//!   see a fault: a throw of one to two cells is metrically identical to a stretched
//!   cell. But the walk already refused exactly those adjacencies, so drop every
//!   triangle that uses one.
//!
//! Points are never moved. The walk's `(block, i, j)` labels stay on the shell.
//! Spec: `surface_tin_fallback_spec` on the planning graph.

use crate::core::attribute::{
    check_metadata_name, validate_attribute_values, AttributeLane, AttributeMetadata,
};
use crate::core::shell::MeshShell;
use crate::core::topology::GridDetection;
use crate::core::{PointSet, PolygonSet};
use crate::foundation::{BBox, GeoError, HasHistory, OperationHistory, Result, Stats};
use indexmap::IndexMap;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// Default maximum link length, in cells of the normalized grid frame.
pub const DEFAULT_MAX_LINK: f64 = 1.8;
/// Below the unit cell's diagonal every quad diagonal is cut and the mesh shreds.
const MIN_LINK: f64 = std::f64::consts::SQRT_2;
/// At or above two cells a triangle skips a node.
const MAX_LINK: f64 = 2.0;
/// Triangles a connected island needs to count as a piece of surface.
const MIN_ISLAND: usize = 8;

/// An unstructured triangulated surface: shared mesh shell + per-node lanes.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(try_from = "TriSurfaceData")]
pub struct TriSurface {
    shell: Arc<MeshShell>,
    /// Primary per-node values — z, in the same order as the shell's nodes.
    values: Vec<f64>,
    #[serde(default)]
    primary_metadata: Option<AttributeMetadata>,
    attributes: IndexMap<String, AttributeLane<Vec<f64>>>,
    #[serde(default)]
    history: OperationHistory,
}

/// Serialized shape: the shell **once**, then N property lanes referencing it.
/// Deserialization routes through the validating constructor.
#[derive(serde::Serialize, serde::Deserialize)]
struct TriSurfaceData {
    shell: MeshShell,
    values: Vec<f64>,
    #[serde(default)]
    primary_metadata: Option<AttributeMetadata>,
    #[serde(default)]
    attributes: IndexMap<String, AttributeLane<Vec<f64>>>,
    #[serde(default)]
    history: OperationHistory,
}

impl TryFrom<TriSurfaceData> for TriSurface {
    type Error = GeoError;
    fn try_from(mut d: TriSurfaceData) -> Result<TriSurface> {
        if let Some(metadata) = &mut d.primary_metadata {
            metadata.migrate_persisted_text();
        }
        for lane in d.attributes.values_mut() {
            lane.metadata.migrate_persisted_text();
        }
        let mut out = TriSurface::from_shell(Arc::new(d.shell), d.values)?;
        if let Some(metadata) = &d.primary_metadata {
            metadata.validate()?;
            validate_attribute_values(metadata, out.values.iter())?;
        }
        out.primary_metadata = d.primary_metadata;
        for (name, lane) in d.attributes {
            out.set_attr_with_metadata(&name, lane.values, lane.metadata)?;
        }
        out.history = d.history;
        Ok(out)
    }
}

impl TriSurface {
    /// Build a surface over an existing (shared) shell. The value lane must
    /// have one value per shell node.
    pub fn from_shell(shell: Arc<MeshShell>, values: Vec<f64>) -> Result<TriSurface> {
        check_lane(&shell, &values, "TriSurface::from_shell")?;
        Ok(TriSurface {
            shell,
            values,
            primary_metadata: None,
            attributes: IndexMap::new(),
            history: OperationHistory::from_entry("tri_surface.from_shell"),
        })
    }

    /// Stable kind label for dispatch/reporting.
    pub fn kind(&self) -> &'static str {
        "tri_surface"
    }

    /// The geometry shell (shared; never copied per property lane).
    pub fn shell(&self) -> &Arc<MeshShell> {
        &self.shell
    }

    /// The surface's vertices as `(x, y, z)` — the shell's XY zipped with the
    /// primary values. Exactly the input points of the triangulation, unmoved.
    pub fn points(&self) -> Vec<[f64; 3]> {
        self.shell
            .nodes()
            .iter()
            .zip(&self.values)
            .map(|(n, z)| [n[0], n[1], *z])
            .collect()
    }

    /// The primary per-node value lane (z). `NaN` = undefined.
    pub fn values(&self) -> &[f64] {
        &self.values
    }

    /// A named attribute lane, if present.
    pub fn attr(&self, name: &str) -> Option<&[f64]> {
        self.attributes.get(name).map(|lane| lane.values.as_slice())
    }

    pub fn attr_metadata(&self, name: &str) -> Option<&AttributeMetadata> {
        self.attributes.get(name).map(|lane| &lane.metadata)
    }

    pub fn primary_metadata(&self) -> Option<&AttributeMetadata> {
        self.primary_metadata.as_ref()
    }

    pub(crate) fn set_primary_metadata(&mut self, metadata: Option<AttributeMetadata>) {
        self.primary_metadata = metadata;
    }

    /// Set (or replace) a named attribute lane (one value per shell node).
    pub fn set_attr(&mut self, name: &str, values: Vec<f64>) -> Result<()> {
        check_lane(&self.shell, &values, "TriSurface::set_attr")?;
        if let Some(existing) = self.attributes.get_mut(name) {
            validate_attribute_values(&existing.metadata, values.iter())?;
            existing.values = values;
        } else {
            self.attributes.insert(
                name.to_string(),
                AttributeLane::new(AttributeMetadata::continuous(name)?, values)?,
            );
        }
        self.record_history(format!("tri_surface.set_attr(name={name})"));
        Ok(())
    }

    pub fn set_attr_with_metadata(
        &mut self,
        name: &str,
        values: Vec<f64>,
        metadata: AttributeMetadata,
    ) -> Result<()> {
        check_lane(&self.shell, &values, "TriSurface::set_attr_with_metadata")?;
        check_metadata_name(name, &metadata)?;
        validate_attribute_values(&metadata, values.iter())?;
        self.attributes
            .insert(name.to_string(), AttributeLane::new(metadata, values)?);
        self.record_history(format!("tri_surface.set_attr_with_metadata(name={name})"));
        Ok(())
    }

    pub fn set_attr_metadata(&mut self, name: &str, metadata: AttributeMetadata) -> Result<()> {
        check_metadata_name(name, &metadata)?;
        let lane = self
            .attributes
            .get_mut(name)
            .ok_or_else(|| GeoError::NotFound(format!("no attribute lane '{name}'")))?;
        validate_attribute_values(&metadata, lane.values.iter())?;
        lane.metadata = metadata;
        self.record_history(format!("tri_surface.set_attr_metadata(name={name})"));
        Ok(())
    }

    /// The names of all attribute lanes, in insertion order.
    pub fn attr_names(&self) -> Vec<&str> {
        self.attributes.keys().map(String::as_str).collect()
    }

    /// Promote an attribute lane to a standalone surface (its primary values)
    /// on the **same shared shell** — no geometry is copied.
    pub fn as_attr_surface(&self, name: &str) -> Option<TriSurface> {
        self.attributes.get(name).map(|lane| TriSurface {
            shell: Arc::clone(&self.shell),
            values: lane.values.clone(),
            primary_metadata: Some(lane.metadata.clone()),
            attributes: IndexMap::new(),
            history: self
                .history
                .with_entry(format!("tri_surface.as_attr_surface(name={name})")),
        })
    }

    /// Triangles, as indices into [`points`](Self::points), counter-clockwise.
    pub fn triangles(&self) -> &[[u32; 3]] {
        self.shell.triangles()
    }

    /// Unique triangle edges minus interior cell diagonals — the quad-dominant
    /// wireframe of the geometry as a flat empty shell (a full lattice cell
    /// shows as a square, not two triangles). Purely topological, never a
    /// function of z. See [`MeshShell::wireframe_edges`].
    ///
    /// `stride = Some(k)` (k ≥ 2) returns the coarse-LOD lattice wireframe
    /// (every k-th grid line per block, outline + seams + fringe kept);
    /// `None`/`Some(1)` is the full wireframe. Display-only — geometry is never
    /// decimated.
    pub fn wireframe_edges(&self, stride: Option<usize>) -> Vec<[u32; 2]> {
        self.shell.wireframe_edges(stride)
    }

    /// Outer boundary ring(s) of the retained triangles.
    pub fn edge(&self) -> &PolygonSet {
        self.shell.edge()
    }

    /// Statistics over the primary values (z).
    pub fn stats(&self) -> Stats {
        Stats::of(&self.values)
    }

    /// Axis-aligned bounding box over the vertices' XY.
    pub fn bbox(&self) -> BBox {
        self.shell.bbox()
    }

    /// The vertices as a `PointSet` — exact, nothing resampled.
    pub fn to_points(&self) -> PointSet {
        let mut out = PointSet::from_coords(self.points());
        *out.operation_history_mut() = self.history.clone();
        out.record_history("tri_surface.to_points()");
        out
    }

    /// Connected components. More than one means the surface is fault-cut and the
    /// mesh honours the fault rather than bridging it.
    pub fn components(&self) -> usize {
        self.shell.components()
    }

    /// Fit a regular grid to the shell (the lossy downward conversion);
    /// errors when the mesh is not regular. Delegates to
    /// [`MeshShell::infer_grid`].
    pub fn infer_grid(&self, tolerance: f64) -> Result<crate::foundation::GridGeometry> {
        self.shell.infer_grid(tolerance)
    }

    /// Human-readable operation history.
    pub fn history(&self) -> &[String] {
        self.history.entries()
    }

    pub(crate) fn set_history(&mut self, history: impl Into<OperationHistory>) {
        self.history = history.into();
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct TriSurfaceDataV1 {
    shell: MeshShell,
    values: Vec<f64>,
    #[serde(default)]
    attributes: IndexMap<String, Vec<f64>>,
    #[serde(default)]
    history: OperationHistory,
}

impl TriSurface {
    pub(crate) fn from_v1_payload(bytes: &[u8]) -> Result<Self> {
        let old: TriSurfaceDataV1 = crate::io::serial::from_bytes(bytes)?;
        let mut out = TriSurface::from_shell(Arc::new(old.shell), old.values)?;
        for (name, values) in old.attributes {
            out.set_attr(&name, values)?;
        }
        out.history = old.history;
        Ok(out)
    }
}

impl HasHistory for TriSurface {
    fn operation_history(&self) -> &OperationHistory {
        &self.history
    }
    fn operation_history_mut(&mut self) -> &mut OperationHistory {
        &mut self.history
    }
}

fn check_lane(shell: &MeshShell, lane: &[f64], ctx: &str) -> Result<()> {
    if lane.len() != shell.n_nodes() {
        return Err(GeoError::GeometryMismatch(format!(
            "{ctx}: {} lane values for {} shell nodes",
            lane.len(),
            shell.n_nodes()
        )));
    }
    Ok(())
}

impl PointSet {
    /// Triangulate the points into a [`TriSurface`] — the fallback when
    /// [`detect_topology`](Self::detect_topology) cannot verify a structured mesh.
    ///
    /// `max_link` is the longest triangle edge to keep, **in cells** of the detected
    /// grid; it must lie in `(√2, 2)`. `None` uses [`DEFAULT_MAX_LINK`].
    ///
    /// `max_bridge` (also **in cells**, `>= max_link`) opt-in relaxes the closed-lattice
    /// rules exactly where the geometry does not close: an edge the strict rules reject —
    /// an unlabelled boundary-fringe node, a fault-seam adjacency the walk refused, an
    /// interior data gap — is admitted anyway when it is no longer than `max_bridge`.
    /// The interior lattice keeps the strict rules regardless. `None` (the default)
    /// keeps the mesh strictly lattice-closed: no fault is bridged, and triangles using
    /// an adjacency the topology walk refused are dropped. Large values readmit the
    /// spanning slivers a Delaunay hull throws across concave bays — keep it near the
    /// gap size being closed.
    pub fn to_tri_surface(
        &self,
        max_link: Option<f64>,
        max_bridge: Option<f64>,
    ) -> Result<TriSurface> {
        let max_link = max_link.unwrap_or(DEFAULT_MAX_LINK);
        if !(MIN_LINK..MAX_LINK).contains(&max_link) {
            return Err(GeoError::GeometryInference(format!(
                "max_link must lie in ({MIN_LINK:.4}, {MAX_LINK}) cells: below the cell diagonal \
                 the mesh shreds, at two cells a triangle skips a node (got {max_link})"
            )));
        }
        if let Some(b) = max_bridge {
            if !b.is_finite() || b < max_link {
                return Err(GeoError::GeometryInference(format!(
                    "max_bridge must be a finite length in cells >= max_link ({max_link}), \
                     got {b}"
                )));
            }
        }

        let d = self.detect_grid(None)?;
        let normalized = normalize(&d);
        let faces = delaunay(&normalized)?;

        // Every adjacency the walk refused straddles a fault. The strict rules never
        // triangulate across one; `max_bridge` admits it like any other open seam.
        let mut forbidden: HashSet<(usize, usize)> = HashSet::new();
        for &(a, b) in &d.frontier {
            forbidden.insert(ordered(a, b));
        }

        let max2 = max_link * max_link;
        let bridge2 = max_bridge.map(|b| b * b);
        let kept: Vec<[usize; 3]> = faces
            .into_iter()
            .filter(|t| {
                let e = [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])];
                e.iter().all(|&(a, b)| {
                    let d2 = sq_dist(normalized[a], normalized[b]);
                    let strict = !forbidden.contains(&ordered(a, b))
                        && index_adjacent(&d, a, b)
                        && d2 <= max2;
                    strict || bridge2.is_some_and(|b2| d2 <= b2)
                })
            })
            .collect();
        if kept.is_empty() {
            return Err(GeoError::GeometryInference(
                "triangulation retained no triangles at the requested max_link".into(),
            ));
        }

        let kept = drop_small_islands(&kept);
        let (nodes, zs, triangles, labels) = compact(&kept, &d);
        let shell = MeshShell::from_triangles(nodes, triangles, labels)?;

        let mut out = TriSurface::from_shell(Arc::new(shell), zs)?;
        *out.operation_history_mut() = self.operation_history().clone();
        out.record_history(match max_bridge {
            Some(b) => format!("points.to_tri_surface(max_link={max_link}, max_bridge={b})"),
            None => format!("points.to_tri_surface(max_link={max_link})"),
        });
        Ok(out)
    }
}

/// Points in the grid frame, each axis divided by its own step, so the cell is a unit
/// square and one scalar `max_link` bounds both axes.
fn normalize(d: &GridDetection) -> Vec<[f64; 2]> {
    d.pts
        .iter()
        .map(|p| {
            let u = p[0] * d.e1[0] + p[1] * d.e1[1];
            let v = p[0] * d.e2[0] + p[1] * d.e2[1];
            [u / d.inc_i, v / d.inc_j]
        })
        .collect()
}

fn sq_dist(a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)
}

fn ordered(a: usize, b: usize) -> (usize, usize) {
    if a <= b {
        (a, b)
    } else {
        (b, a)
    }
}

/// May `a` and `b` share a triangle edge?
///
/// The walk labels every node with a fault block and a `(column, row)` inside it, so
/// this is decidable: nodes in the same block are grid neighbours when their indices
/// differ by one, and nodes in *different* blocks have no grid adjacency at all — the
/// walk refused to connect them, which is precisely what a fault is. A link length
/// cannot see this, because a one-cell fault throw is metrically identical to a
/// stretched cell: 36 of the 84 bridges on the reference surface are shorter than a
/// legal quad diagonal.
fn index_adjacent(d: &GridDetection, a: usize, b: usize) -> bool {
    let (Some(&(ba, ia, ja)), Some(&(bb, ib, jb))) = (d.index_of.get(&a), d.index_of.get(&b))
    else {
        return true;
    };
    // Different fault blocks: the walk could not connect them, so no grid adjacency
    // exists, and every such edge crosses a fault. Trust even a one-node block —
    // requiring a minimum block size to believe this was tried, and left 64 of the 84
    // bridges standing, because the fault-trace nodes are exactly the snapped ones the
    // walk cannot chain into a block.
    if ba != bb {
        return false;
    }
    (ia - ib).abs().max((ja - jb).abs()) == 1
}

/// Delaunay over the normalized points, returning faces as indices into `pts`.
fn delaunay(pts: &[[f64; 2]]) -> Result<Vec<[usize; 3]>> {
    use spade::{DelaunayTriangulation, Point2, Triangulation};

    let mut tri: DelaunayTriangulation<Point2<f64>> = DelaunayTriangulation::new();
    let mut of_handle: HashMap<usize, usize> = HashMap::new();
    for (i, p) in pts.iter().enumerate() {
        let h = tri.insert(Point2::new(p[0], p[1])).map_err(|e| {
            GeoError::GeometryInference(format!("triangulation rejected a point: {e}"))
        })?;
        of_handle.insert(h.index(), i);
    }

    let mut faces = Vec::new();
    for f in tri.inner_faces() {
        let v = f.vertices();
        let (a, b, c) = (
            of_handle[&v[0].fix().index()],
            of_handle[&v[1].fix().index()],
            of_handle[&v[2].fix().index()],
        );
        faces.push([a, b, c]);
    }
    if faces.is_empty() {
        return Err(GeoError::GeometryInference(
            "triangulation produced no triangles (are the points collinear?)".into(),
        ));
    }
    Ok(faces)
}

/// Drop islands too small to be a piece of surface.
///
/// NOT "keep only the largest component". A fault-cut surface genuinely has more than
/// one block, and once the fault constraints stop the mesh bridging the throw, those
/// blocks separate — dropping all but the biggest would silently discard real data (on
/// the reference surface, a whole fault-bounded lobe: 1,990 nodes). What we do want
/// gone are the specks a length filter leaves behind at the clip boundary.
fn drop_small_islands(tris: &[[usize; 3]]) -> Vec<[usize; 3]> {
    let mut parent: HashMap<usize, usize> = HashMap::new();
    fn find(parent: &mut HashMap<usize, usize>, x: usize) -> usize {
        let mut root = x;
        while let Some(&p) = parent.get(&root) {
            if p == root {
                break;
            }
            root = p;
        }
        let mut cur = x;
        while let Some(&p) = parent.get(&cur) {
            if p == cur {
                break;
            }
            parent.insert(cur, root);
            cur = p;
        }
        root
    }
    for t in tris {
        for &v in t {
            parent.entry(v).or_insert(v);
        }
    }
    for t in tris {
        let r0 = find(&mut parent, t[0]);
        for &v in &t[1..] {
            let r = find(&mut parent, v);
            if r != r0 {
                parent.insert(r, r0);
            }
        }
    }
    let mut size: HashMap<usize, usize> = HashMap::new();
    for t in tris {
        *size.entry(find(&mut parent, t[0])).or_insert(0) += 1;
    }
    // `MIN_ISLAND` distinguishes clip-boundary specks from an established surface;
    // it is not a minimum valid surface size. If the whole triangulation is small,
    // discarding every component would turn valid three-to-five-point clouds into an
    // empty mesh and make boundary extraction report the misleading "no boundary".
    // Preserve all components unless there is a larger surface for tiny islands to be
    // classified relative to.
    if !size.values().any(|&n| n >= MIN_ISLAND) {
        return tris.to_vec();
    }
    tris.iter()
        .copied()
        .filter(|t| size[&find(&mut parent, t[0])] >= MIN_ISLAND)
        .collect()
}

/// Reindex the surviving triangles onto only the vertices they use, splitting XY
/// (shell) from z (property lane) and carrying each vertex's `(block, i, j)` walk
/// label (`None` where the walk left it unlabelled).
#[allow(clippy::type_complexity)]
fn compact(
    tris: &[[usize; 3]],
    d: &GridDetection,
) -> (
    Vec<[f64; 2]>,
    Vec<f64>,
    Vec<[u32; 3]>,
    Vec<Option<crate::core::shell::WalkLabel>>,
) {
    let mut remap: HashMap<usize, u32> = HashMap::new();
    let mut nodes = Vec::new();
    let mut zs = Vec::new();
    let mut labels = Vec::new();
    for t in tris {
        for &v in t {
            remap.entry(v).or_insert_with(|| {
                nodes.push([d.pts[v][0], d.pts[v][1]]);
                zs.push(d.zs[v]);
                labels.push(d.index_of.get(&v).copied());
                (nodes.len() - 1) as u32
            });
        }
    }
    let triangles = tris
        .iter()
        .map(|t| [remap[&t[0]], remap[&t[1]], remap[&t[2]]])
        .collect();
    (nodes, zs, triangles, labels)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lattice(ncol: usize, nrow: usize, xinc: f64, yinc: f64, az_deg: f64) -> Vec<[f64; 3]> {
        let (s, c) = az_deg.to_radians().sin_cos();
        let mut out = Vec::new();
        for j in 0..nrow {
            for i in 0..ncol {
                let (u, v) = (xinc * i as f64, yinc * j as f64);
                out.push([1000.0 + u * c - v * s, 2000.0 + u * s + v * c, -1800.0]);
            }
        }
        out
    }

    #[test]
    fn triangulates_a_full_grid_into_one_sheet() {
        let coords = lattice(9, 7, 50.0, 50.0, 25.0);
        let tin = PointSet::from_coords(coords.clone())
            .to_tri_surface(None, None)
            .unwrap();
        assert_eq!(tin.kind(), "tri_surface");
        assert_eq!(tin.points().len(), coords.len());
        // A fully populated m x n lattice triangulates into 2*(m-1)*(n-1) quads' worth.
        assert_eq!(tin.triangles().len(), 2 * 8 * 6);
        assert_eq!(tin.edge().rings().len(), 1);
        // The vertices are the input, unmoved.
        let mut before: Vec<[u64; 3]> = coords.iter().map(bits).collect();
        let mut after: Vec<[u64; 3]> = tin.points().iter().map(bits).collect();
        before.sort();
        after.sort();
        assert_eq!(before, after);
        // The walk labels stay on the shell — one per node.
        assert_eq!(tin.shell().labels().len(), tin.points().len());
        assert!(tin.shell().labels().iter().all(Option::is_some));
    }

    #[test]
    fn keeps_complete_small_scattered_surfaces() {
        let clouds = [
            vec![
                [0.0, 0.0, -2600.0],
                [10.0, 1.0, -2610.0],
                [2.0, 9.0, -2620.0],
                [12.0, 11.0, -2630.0],
                [6.0, 5.0, -2615.0],
            ],
            vec![
                [0.0, 0.0, -2600.0],
                [10.0, 1.0, -2610.0],
                [2.0, 9.0, -2620.0],
                [12.0, 11.0, -2630.0],
            ],
        ];

        for coords in clouds {
            for max_bridge in [None, Some(3.4)] {
                let tin = PointSet::from_coords(coords.clone())
                    .to_tri_surface(None, max_bridge)
                    .unwrap();
                assert_eq!(tin.points().len(), coords.len());
                assert!(!tin.triangles().is_empty());
                assert_eq!(tin.components(), 1);
                assert_eq!(tin.edge().rings().len(), 1);
                assert_eq!(tin.shell().labels().len(), coords.len());
            }
        }
    }

    fn bits(c: &[f64; 3]) -> [u64; 3] {
        [c[0].to_bits(), c[1].to_bits(), c[2].to_bits()]
    }

    #[test]
    fn anisotropic_cells_need_the_normalized_frame() {
        // A 50 x 20 cell has a diagonal (53.9 m) longer than two short steps (40 m), so
        // no *world-unit* max link can both keep the diagonals and reject a skipped
        // node. In the normalized frame the cell is a unit square and 1.8 works.
        let coords = lattice(9, 7, 50.0, 20.0, 40.0);
        let tin = PointSet::from_coords(coords)
            .to_tri_surface(None, None)
            .unwrap();
        assert_eq!(tin.triangles().len(), 2 * 8 * 6);
        assert_eq!(tin.edge().rings().len(), 1);
    }

    #[test]
    fn does_not_bridge_a_fault() {
        // Two blocks pulled apart and offset: the walk stalls at the gap, and those
        // refused adjacencies must not become triangles.
        let mut coords = Vec::new();
        for j in 0..9 {
            for i in 0..6 {
                coords.push([50.0 * i as f64, 50.0 * j as f64, -1800.0]);
            }
        }
        for j in 0..9 {
            for i in 8..14 {
                coords.push([50.0 * i as f64 + 20.0, 50.0 * j as f64 + 25.0, -1900.0]);
            }
        }
        let p = PointSet::from_coords(coords);
        let (labels, report) = p.detect_topology(None).unwrap();
        assert!(
            labels.is_none() && !report.verified(),
            "the fixture is faulted"
        );

        assert!(report.blocks >= 2, "the fixture must split into blocks");

        let tin = p.to_tri_surface(None, None).unwrap();
        // Both blocks survive. A fault-cut surface really *is* two sheets, and keeping
        // only the largest would silently discard real data. What must not happen is a
        // triangle spanning the throw.
        assert_eq!(tin.components(), 2, "the fault is honoured, not bridged");
        assert!(
            tin.points().len() > 6 * 9,
            "the far block must be kept, not discarded"
        );
    }

    #[test]
    fn is_deterministic() {
        // The boundary trace must not depend on hash iteration order. An undirected
        // trace has to guess which way to leave a pinch vertex, and the guess changed
        // the emitted rings from run to run on the same triangles.
        let coords = lattice(11, 9, 50.0, 30.0, 17.0);
        let p = PointSet::from_coords(coords);
        let first = p.to_tri_surface(None, None).unwrap();
        for _ in 0..8 {
            let again = p.to_tri_surface(None, None).unwrap();
            assert_eq!(again.triangles(), first.triangles());
            assert_eq!(again.points(), first.points());
            assert_eq!(again.edge().rings(), first.edge().rings());
        }
    }

    #[test]
    fn max_bridge_closes_fault_seams_and_fringe() {
        // The faulted fixture from `does_not_bridge_a_fault`: two blocks separated by
        // a ~3.4-cell gap. Strict rules keep them apart; max_bridge=4 closes the seam.
        let mut coords = Vec::new();
        for j in 0..9 {
            for i in 0..6 {
                coords.push([50.0 * i as f64, 50.0 * j as f64, -1800.0]);
            }
        }
        for j in 0..9 {
            for i in 8..14 {
                coords.push([50.0 * i as f64 + 20.0, 50.0 * j as f64 + 25.0, -1900.0]);
            }
        }
        let p = PointSet::from_coords(coords);
        assert_eq!(p.to_tri_surface(None, None).unwrap().components(), 2);
        let bridged = p.to_tri_surface(None, Some(4.0)).unwrap();
        assert_eq!(bridged.components(), 1, "the seam closes at max_bridge=4");

        // A fringe point 2.5 cells off the lattice boundary: dropped strictly,
        // attached under max_bridge=3.
        let mut coords = lattice(9, 7, 50.0, 50.0, 0.0);
        coords.push([1000.0 + 8.0 * 50.0 + 125.0, 2000.0 + 150.0, -1800.0]);
        let p = PointSet::from_coords(coords.clone());
        assert_eq!(p.to_tri_surface(None, None).unwrap().points().len(), 63);
        let tin = p.to_tri_surface(None, Some(3.0)).unwrap();
        assert_eq!(tin.points().len(), 64, "the fringe point joins the mesh");
        assert_eq!(tin.components(), 1);
    }

    #[test]
    fn rejects_a_max_bridge_below_max_link() {
        let p = PointSet::from_coords(lattice(6, 6, 50.0, 50.0, 0.0));
        for bad in [0.5, 1.0, f64::NAN, f64::INFINITY] {
            assert!(
                p.to_tri_surface(None, Some(bad)).is_err(),
                "max_bridge {bad} must be rejected"
            );
        }
        assert!(p.to_tri_surface(None, Some(1.8)).is_ok());
    }

    #[test]
    fn wireframe_hides_interior_diagonals_only() {
        // Axis-aligned 9 x 7 lattice, 50 m square cells: the wireframe must be the
        // lattice edges alone — 9*6 verticals + 7*8 horizontals — every one 50 m,
        // while triangles() still carries the diagonals.
        let tin = PointSet::from_coords(lattice(9, 7, 50.0, 50.0, 0.0))
            .to_tri_surface(None, None)
            .unwrap();
        let wf = tin.wireframe_edges(None);
        assert_eq!(wf.len(), 9 * 6 + 7 * 8);
        let pts = tin.points();
        for [a, b] in &wf {
            let (p, q) = (pts[*a as usize], pts[*b as usize]);
            let len = ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2)).sqrt();
            assert!((len - 50.0).abs() < 1e-9, "diagonal survived: {len}");
        }
    }

    #[test]
    fn wireframe_is_shape_independent() {
        // The geometry is a flat empty shell: spiking a node's z kinks four cells,
        // but shape never changes the wireframe — the shell stays all quads.
        // Splitting non-planar cells is the surface layer's concern.
        let flat = lattice(9, 7, 50.0, 50.0, 0.0);
        let mut spiked = flat.clone();
        for c in spiked.iter_mut() {
            if (c[0] - 1200.0).abs() < 1e-9 && (c[1] - 2150.0).abs() < 1e-9 {
                c[2] = -1780.0;
            }
        }
        let wf_flat = PointSet::from_coords(flat)
            .to_tri_surface(None, None)
            .unwrap()
            .wireframe_edges(None);
        let wf_spiked = PointSet::from_coords(spiked)
            .to_tri_surface(None, None)
            .unwrap()
            .wireframe_edges(None);
        assert_eq!(wf_flat.len(), 9 * 6 + 7 * 8);
        assert_eq!(
            wf_flat, wf_spiked,
            "z must not leak into the geometry wireframe"
        );
    }

    #[test]
    fn strided_wireframe_keeps_seams_and_boundary_on_a_faulted_mesh() {
        // The faulted two-block fixture: striding must reduce the interior lattice
        // while keeping every boundary edge and every edge touching an unlabelled
        // fault-trace node (the seam), at every stride.
        let mut coords = Vec::new();
        for j in 0..12 {
            for i in 0..8 {
                coords.push([50.0 * i as f64, 50.0 * j as f64, -1800.0]);
            }
        }
        for j in 0..12 {
            for i in 10..18 {
                coords.push([50.0 * i as f64 + 20.0, 50.0 * j as f64 + 25.0, -1900.0]);
            }
        }
        let tin = PointSet::from_coords(coords)
            .to_tri_surface(None, None)
            .unwrap();
        assert_eq!(tin.components(), 2, "the fixture is faulted");

        let full = tin.wireframe_edges(None);
        let labels = tin.shell().labels();
        // Boundary edges (carried by exactly one triangle) and edges touching an
        // unlabelled node must survive every stride.
        let mut count: std::collections::BTreeMap<(u32, u32), u8> = Default::default();
        for t in tin.triangles() {
            for &(a, b) in &[(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                *count
                    .entry(if a <= b { (a, b) } else { (b, a) })
                    .or_insert(0) += 1;
            }
        }
        for k in [2usize, 4] {
            let wf = tin.wireframe_edges(Some(k));
            let set: HashSet<(u32, u32)> = wf
                .iter()
                .map(|e| ordered(e[0] as usize, e[1] as usize))
                .map(|(a, b)| (a as u32, b as u32))
                .collect();
            assert!(
                wf.len() < full.len(),
                "stride {k} must reduce the wireframe"
            );
            for [a, b] in &full {
                let key = if a <= b { (*a, *b) } else { (*b, *a) };
                let boundary = count.get(&key) == Some(&1);
                let touches_fringe = labels[*a as usize].is_none() || labels[*b as usize].is_none();
                if boundary || touches_fringe {
                    assert!(
                        set.contains(&key),
                        "seam/boundary edge {key:?} dropped at stride {k}"
                    );
                }
            }
        }
    }

    #[test]
    fn attribute_lanes_ride_the_shared_shell() {
        let tin = PointSet::from_coords(lattice(6, 5, 50.0, 50.0, 10.0))
            .to_tri_surface(None, None)
            .unwrap();
        let n = tin.points().len();
        let mut with_amp = tin.clone();
        with_amp
            .set_attr("amp", (0..n).map(|k| k as f64 * 0.25).collect())
            .unwrap();
        assert_eq!(with_amp.attr_names(), vec!["amp"]);
        assert!(with_amp.set_attr("bad", vec![0.0; n + 1]).is_err());

        let promoted = with_amp.as_attr_surface("amp").unwrap();
        assert_eq!(promoted.values()[3], 0.75);
        assert!(Arc::ptr_eq(with_amp.shell(), promoted.shell()));
        // The clone also shares the shell.
        assert!(Arc::ptr_eq(tin.shell(), with_amp.shell()));
    }

    #[test]
    fn serde_round_trips_shell_once_with_lanes() {
        let mut tin = PointSet::from_coords(lattice(5, 4, 50.0, 50.0, 0.0))
            .to_tri_surface(None, None)
            .unwrap();
        let n = tin.points().len();
        tin.set_attr("amp", vec![1.5; n]).unwrap();
        let json = serde_json::to_string(&tin).unwrap();
        assert_eq!(json.matches("\"shell\"").count(), 1);
        let back: TriSurface = serde_json::from_str(&json).unwrap();
        assert_eq!(back.points(), tin.points());
        assert_eq!(back.triangles(), tin.triangles());
        assert_eq!(back.wireframe_edges(None), tin.wireframe_edges(None));
        assert_eq!(back.attr("amp").unwrap(), tin.attr("amp").unwrap());
        assert_eq!(back.shell().labels(), tin.shell().labels());
    }

    #[test]
    fn positional_v1_payload_migrates_attribute_metadata() {
        let tin = PointSet::from_coords(lattice(5, 4, 50.0, 50.0, 0.0))
            .to_tri_surface(None, None)
            .unwrap();
        let mut attributes = IndexMap::new();
        attributes.insert("legacy".into(), vec![2.0; tin.values().len()]);
        let old = TriSurfaceDataV1 {
            shell: (**tin.shell()).clone(),
            values: tin.values().to_vec(),
            attributes,
            history: OperationHistory::from_entry("v1.fixture"),
        };
        let bytes = crate::io::serial::to_bytes(&old).unwrap();
        let migrated = TriSurface::from_v1_payload(&bytes).unwrap();
        assert_eq!(
            migrated.attr_metadata("legacy"),
            Some(&AttributeMetadata::continuous("legacy").unwrap())
        );
    }

    #[test]
    fn deserialize_rejects_invalid_primary_metadata() {
        let tin = PointSet::from_coords(lattice(5, 4, 50.0, 50.0, 0.0))
            .to_tri_surface(None, None)
            .unwrap();
        let bad = TriSurfaceData {
            shell: (**tin.shell()).clone(),
            values: tin.values().to_vec(),
            primary_metadata: Some(AttributeMetadata {
                id: "depth".into(),
                label: "\t".into(),
                kind: crate::AttributeKind::Continuous,
                units: None,
                codes: None,
            }),
            attributes: IndexMap::new(),
            history: OperationHistory::new(),
        };
        let bytes = crate::io::serial::to_bytes(&bad).unwrap();
        assert!(crate::io::serial::from_bytes::<TriSurface>(&bytes).is_err());

        let fractional_categorical = TriSurfaceData {
            shell: (**tin.shell()).clone(),
            values: vec![1.5; tin.values().len()],
            primary_metadata: Some(
                AttributeMetadata::new(
                    "facies",
                    "Facies",
                    crate::AttributeKind::Categorical,
                    None,
                    None,
                )
                .unwrap(),
            ),
            attributes: IndexMap::new(),
            history: OperationHistory::new(),
        };
        let bytes = crate::io::serial::to_bytes(&fractional_categorical).unwrap();
        assert!(crate::io::serial::from_bytes::<TriSurface>(&bytes).is_err());
    }

    #[test]
    fn rejects_a_max_link_outside_the_band() {
        let p = PointSet::from_coords(lattice(6, 6, 50.0, 50.0, 0.0));
        for bad in [1.0, 1.41, 2.0, 2.5] {
            assert!(
                p.to_tri_surface(Some(bad), None).is_err(),
                "max_link {bad} is outside (sqrt2, 2)"
            );
        }
        assert!(p.to_tri_surface(Some(1.8), None).is_ok());
    }
}
