//! `TriSurface` — the fallback surface for points whose `(column, row)` topology
//! cannot be verified.
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
//! Points are never moved. Spec: `surface_tin_fallback_spec` on the planning graph.

use crate::core::topology::{GridDetection, NodeIndex};
use crate::core::{PointSet, PolygonSet};
use crate::foundation::{BBox, GeoError, HasHistory, OperationHistory, Result, Stats};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

/// Default maximum link length, in cells of the normalized grid frame.
pub const DEFAULT_MAX_LINK: f64 = 1.8;
/// Below the unit cell's diagonal every quad diagonal is cut and the mesh shreds.
const MIN_LINK: f64 = std::f64::consts::SQRT_2;
/// At or above two cells a triangle skips a node.
const MAX_LINK: f64 = 2.0;
/// Triangles a connected island needs to count as a piece of surface.
const MIN_ISLAND: usize = 8;

/// An unstructured triangulated surface over the original points.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct TriSurface {
    points: Vec<[f64; 3]>,
    triangles: Vec<[u32; 3]>,
    edge: PolygonSet,
    /// Unique triangle edges minus interior cell diagonals (see
    /// [`wireframe_edges`](Self::wireframe_edges)). Empty on legacy payloads.
    #[serde(default)]
    wireframe: Vec<[u32; 2]>,
    #[serde(default)]
    history: OperationHistory,
}

impl TriSurface {
    /// Stable kind label for dispatch/reporting.
    pub fn kind(&self) -> &'static str {
        "tri_surface"
    }

    /// The surface's vertices, exactly as they came in.
    pub fn points(&self) -> &[[f64; 3]] {
        &self.points
    }

    /// Triangles, as indices into [`points`](Self::points), counter-clockwise.
    pub fn triangles(&self) -> &[[u32; 3]] {
        &self.triangles
    }

    /// Outer boundary ring(s) of the retained triangles.
    pub fn edge(&self) -> &PolygonSet {
        &self.edge
    }

    /// Unique triangle edges with flat interior cell diagonals removed — the
    /// quad-dominant wireframe a structured-surface display draws (a flat
    /// lattice cell shows as a square, not two triangles).
    ///
    /// A diagonal — an edge whose endpoints the topology walk labelled
    /// `(±1, ±1)` apart within one fault block — is dropped only when both of
    /// its triangles survived *and* the quad is planar within a relative
    /// tolerance; a kinked cell keeps its triangles visible, and a diagonal on
    /// the mesh boundary keeps its lone triangle drawable. Legacy payloads
    /// stored before the classification fall back to every unique edge.
    pub fn wireframe_edges(&self) -> Vec<[u32; 2]> {
        if !self.wireframe.is_empty() {
            return self.wireframe.clone();
        }
        let mut seen: BTreeSet<(u32, u32)> = BTreeSet::new();
        for t in &self.triangles {
            for &(a, b) in &[(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                seen.insert(if a <= b { (a, b) } else { (b, a) });
            }
        }
        seen.into_iter().map(|(a, b)| [a, b]).collect()
    }

    /// Statistics over the vertices' z.
    pub fn stats(&self) -> Stats {
        Stats::of(&self.points.iter().map(|p| p[2]).collect::<Vec<_>>())
    }

    /// Axis-aligned bounding box over the vertices' XY.
    pub fn bbox(&self) -> BBox {
        let mut b = BBox {
            xmin: f64::INFINITY,
            ymin: f64::INFINITY,
            xmax: f64::NEG_INFINITY,
            ymax: f64::NEG_INFINITY,
        };
        for p in &self.points {
            b.xmin = b.xmin.min(p[0]);
            b.xmax = b.xmax.max(p[0]);
            b.ymin = b.ymin.min(p[1]);
            b.ymax = b.ymax.max(p[1]);
        }
        b
    }

    /// The vertices as a `PointSet` — exact, nothing resampled.
    pub fn to_points(&self) -> PointSet {
        let mut out = PointSet::from_coords(self.points.clone());
        *out.operation_history_mut() = self.history.clone();
        out.record_history("tri_surface.to_points()");
        out
    }

    /// Connected components. More than one means the surface is fault-cut and the
    /// mesh honours the fault rather than bridging it.
    pub fn components(&self) -> usize {
        let tris: Vec<[usize; 3]> = self
            .triangles
            .iter()
            .map(|t| [t[0] as usize, t[1] as usize, t[2] as usize])
            .collect();
        count_components(&tris)
    }

    /// Human-readable operation history.
    pub fn history(&self) -> &[String] {
        self.history.entries()
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
        let (points, triangles, labels) = compact(&kept, &d);
        let edge = boundary_rings(&triangles, &points)?;
        let wireframe = wireframe(&triangles, &labels, &points);

        let mut out = TriSurface {
            points,
            triangles,
            edge,
            wireframe,
            history: OperationHistory::new(),
        };
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
    tris.iter()
        .copied()
        .filter(|t| size[&find(&mut parent, t[0])] >= MIN_ISLAND)
        .collect()
}

/// Reindex the surviving triangles onto only the vertices they use, carrying each
/// vertex's `(block, i, j)` walk label (`None` where the walk left it unlabelled).
#[allow(clippy::type_complexity)]
fn compact(
    tris: &[[usize; 3]],
    d: &GridDetection,
) -> (Vec<[f64; 3]>, Vec<[u32; 3]>, Vec<Option<NodeIndex>>) {
    let mut remap: HashMap<usize, u32> = HashMap::new();
    let mut points = Vec::new();
    let mut labels = Vec::new();
    for t in tris {
        for &v in t {
            remap.entry(v).or_insert_with(|| {
                points.push([d.pts[v][0], d.pts[v][1], d.zs[v]]);
                labels.push(d.index_of.get(&v).copied());
                (points.len() - 1) as u32
            });
        }
    }
    let triangles = tris
        .iter()
        .map(|t| [remap[&t[0]], remap[&t[1]], remap[&t[2]]])
        .collect();
    (points, triangles, labels)
}

/// Off-plane offset of a quad's fourth corner, relative to its diagonal length,
/// above which the cell is not flat and its two triangles stay visible. On the
/// faulted reference surface, smooth structural curvature sits far below this
/// (p50 ≈ 0) while fault-drag kinks sit well above it.
const PLANAR_REL_TOL: f64 = 0.05;

/// Unique triangle edges minus **flat** interior cell diagonals — the
/// quad-dominant wireframe. A diagonal is hidden only when both triangles of
/// its cell survived (the edge is carried twice) *and* the quad is planar
/// within [`PLANAR_REL_TOL`]; a kinked cell keeps its triangles visible, and a
/// boundary diagonal stays so its lone triangle keeps all three sides.
fn wireframe(
    triangles: &[[u32; 3]],
    labels: &[Option<NodeIndex>],
    points: &[[f64; 3]],
) -> Vec<[u32; 2]> {
    // For each undirected edge, the opposite vertex of every triangle carrying it.
    let mut opposite: BTreeMap<(u32, u32), Vec<u32>> = BTreeMap::new();
    for t in triangles {
        for &(a, b, o) in &[(t[0], t[1], t[2]), (t[1], t[2], t[0]), (t[2], t[0], t[1])] {
            let key = if a <= b { (a, b) } else { (b, a) };
            opposite.entry(key).or_default().push(o);
        }
    }
    let mut out = Vec::new();
    for ((a, b), opp) in opposite {
        let hide = opp.len() == 2
            && is_cell_diagonal(labels[a as usize], labels[b as usize])
            && quad_is_planar(
                points[a as usize],
                points[b as usize],
                points[opp[0] as usize],
                points[opp[1] as usize],
            );
        if !hide {
            out.push([a, b]);
        }
    }
    out
}

/// Is the quad split by diagonal `(a, b)` flat? The offset of `o2` from the
/// plane through `(a, b, o1)` must stay within [`PLANAR_REL_TOL`] of the
/// diagonal's length. Tilt alone never trips this — a planar dipping cell has
/// zero offset; only twist/kink across the cell does.
fn quad_is_planar(a: [f64; 3], b: [f64; 3], o1: [f64; 3], o2: [f64; 3]) -> bool {
    let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let ao1 = [o1[0] - a[0], o1[1] - a[1], o1[2] - a[2]];
    let ao2 = [o2[0] - a[0], o2[1] - a[1], o2[2] - a[2]];
    let n = [
        ab[1] * ao1[2] - ab[2] * ao1[1],
        ab[2] * ao1[0] - ab[0] * ao1[2],
        ab[0] * ao1[1] - ab[1] * ao1[0],
    ];
    let nn = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    let diag = (ab[0] * ab[0] + ab[1] * ab[1] + ab[2] * ab[2]).sqrt();
    if nn <= f64::EPSILON || diag <= f64::EPSILON {
        return true; // degenerate triangle: nothing meaningful to keep visible
    }
    let offset = (n[0] * ao2[0] + n[1] * ao2[1] + n[2] * ao2[2]).abs() / nn;
    offset <= PLANAR_REL_TOL * diag
}

/// Are `a` and `b` opposite corners of one lattice cell in the same fault block?
fn is_cell_diagonal(a: Option<NodeIndex>, b: Option<NodeIndex>) -> bool {
    match (a, b) {
        (Some((ba, ia, ja)), Some((bb, ib, jb))) => {
            ba == bb && (ia - ib).abs() == 1 && (ja - jb).abs() == 1
        }
        _ => false,
    }
}

/// Chain the boundary edges into closed rings — the surface's outer boundary and the
/// outline of any interior hole.
///
/// The edges are **directed**, taken from each triangle's counter-clockwise winding: a
/// boundary edge is one whose reverse no triangle carries. Direction matters. An
/// undirected trace has to guess which way to leave a pinch vertex — a vertex where
/// the boundary touches itself — and the guess depends on hash iteration order, so the
/// same triangles yield different rings from run to run. Following the winding leaves
/// exactly one outgoing edge to take.
fn boundary_rings(tris: &[[u32; 3]], points: &[[f64; 3]]) -> Result<PolygonSet> {
    let directed: HashSet<(u32, u32)> = tris
        .iter()
        .flat_map(|t| [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])])
        .collect();
    let mut out: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    let mut remaining = 0usize;
    for &(a, b) in &directed {
        if !directed.contains(&(b, a)) {
            out.entry(a).or_default().push(b);
            remaining += 1;
        }
    }
    if remaining == 0 {
        return Err(GeoError::GeometryInference(
            "triangulated surface has no boundary".into(),
        ));
    }
    // Deterministic choice at a pinch vertex, where more than one boundary edge leaves.
    for vs in out.values_mut() {
        vs.sort_unstable();
    }
    let total = remaining;

    let mut rings = Vec::new();
    while let Some((&start, _)) = out.iter().find(|(_, vs)| !vs.is_empty()) {
        let mut ring = vec![start];
        let mut current = start;
        loop {
            let next = match out.get_mut(&current).and_then(|vs| vs.pop()) {
                Some(v) => v,
                None => {
                    return Err(GeoError::GeometryInference(
                        "triangulated surface boundary is not closed".into(),
                    ))
                }
            };
            if next == start {
                break;
            }
            ring.push(next);
            current = next;
            if ring.len() > total + 1 {
                return Err(GeoError::GeometryInference(
                    "triangulated surface boundary tracing did not close".into(),
                ));
            }
        }
        if ring.len() >= 3 {
            ring.push(start); // close it
            rings.push(
                ring.into_iter()
                    .map(|v| [points[v as usize][0], points[v as usize][1], 0.0])
                    .collect(),
            );
        }
    }
    if rings.is_empty() {
        return Err(GeoError::GeometryInference(
            "triangulated surface produced no closed boundary".into(),
        ));
    }
    Ok(PolygonSet::from_rings(rings))
}

/// Number of connected components among `tris`.
fn count_components(tris: &[[usize; 3]]) -> usize {
    let mut parent: HashMap<usize, usize> = HashMap::new();
    fn find(parent: &mut HashMap<usize, usize>, x: usize) -> usize {
        let mut root = x;
        while let Some(&p) = parent.get(&root) {
            if p == root {
                break;
            }
            root = p;
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
    let roots: HashSet<usize> = tris.iter().map(|t| find(&mut parent, t[0])).collect();
    roots.len()
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
        let wf = tin.wireframe_edges();
        assert_eq!(wf.len(), 9 * 6 + 7 * 8);
        for [a, b] in &wf {
            let (p, q) = (tin.points()[*a as usize], tin.points()[*b as usize]);
            let len = ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2)).sqrt();
            assert!((len - 50.0).abs() < 1e-9, "diagonal survived: {len}");
        }
    }

    #[test]
    fn wireframe_keeps_triangles_in_non_planar_cells() {
        // Flat 9 x 7 lattice with one interior node spiked 20 m: the four cells
        // around it are kinked far past the planarity tolerance, so their
        // diagonals stay visible while the rest of the sheet stays quads.
        let mut coords = lattice(9, 7, 50.0, 50.0, 0.0);
        for c in coords.iter_mut() {
            if (c[0] - 1200.0).abs() < 1e-9 && (c[1] - 2150.0).abs() < 1e-9 {
                c[2] = -1780.0;
            }
        }
        let tin = PointSet::from_coords(coords)
            .to_tri_surface(None, None)
            .unwrap();
        let lattice_only = 9 * 6 + 7 * 8;
        let extra = tin.wireframe_edges().len() - lattice_only;
        assert_eq!(extra, 4, "the four kinked cells keep their diagonals");
    }

    #[test]
    fn wireframe_falls_back_to_all_edges_on_legacy_payloads() {
        let tin = PointSet::from_coords(lattice(9, 7, 50.0, 50.0, 0.0))
            .to_tri_surface(None, None)
            .unwrap();
        let mut v = serde_json::to_value(&tin).unwrap();
        v.as_object_mut().unwrap().remove("wireframe");
        let legacy: TriSurface = serde_json::from_value(v).unwrap();
        // 2*8*6 triangles: (3*96 + 28 boundary)/2 = 158 unique edges.
        assert_eq!(legacy.wireframe_edges().len(), 158);
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
