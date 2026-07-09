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

use crate::core::topology::GridDetection;
use crate::core::{PointSet, PolygonSet};
use crate::foundation::{BBox, GeoError, HasHistory, OperationHistory, Result, Stats};
use std::collections::{BTreeMap, HashMap, HashSet};

/// Default maximum link length, in cells of the normalized grid frame.
pub const DEFAULT_MAX_LINK: f64 = 1.8;
/// Below the unit cell's diagonal every quad diagonal is cut and the mesh shreds.
const MIN_LINK: f64 = std::f64::consts::SQRT_2;
/// At or above two cells a triangle skips a node.
const MAX_LINK: f64 = 2.0;

/// An unstructured triangulated surface over the original points.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct TriSurface {
    points: Vec<[f64; 3]>,
    triangles: Vec<[u32; 3]>,
    edge: PolygonSet,
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
    /// grid; it must lie in `(√2, 2)`. `None` uses [`DEFAULT_MAX_LINK`]. The result is
    /// a single connected component: triangles outside the largest one are dropped,
    /// as are triangles that bridge a fault (those using an adjacency the topology
    /// walk refused).
    pub fn to_tri_surface(&self, max_link: Option<f64>) -> Result<TriSurface> {
        let max_link = max_link.unwrap_or(DEFAULT_MAX_LINK);
        if !(MIN_LINK..MAX_LINK).contains(&max_link) {
            return Err(GeoError::GeometryInference(format!(
                "max_link must lie in ({MIN_LINK:.4}, {MAX_LINK}) cells: below the cell diagonal \
                 the mesh shreds, at two cells a triangle skips a node (got {max_link})"
            )));
        }

        let d = self.detect_grid(None)?;
        let normalized = normalize(&d);
        let faces = delaunay(&normalized)?;

        // Every adjacency the walk refused straddles a fault. Never triangulate across one.
        let mut forbidden: HashSet<(usize, usize)> = HashSet::new();
        for &(a, b) in &d.frontier {
            forbidden.insert(ordered(a, b));
        }

        let max2 = max_link * max_link;
        let kept: Vec<[usize; 3]> = faces
            .into_iter()
            .filter(|t| {
                let e = [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])];
                e.iter().all(|&(a, b)| {
                    !forbidden.contains(&ordered(a, b))
                        && index_adjacent(&d, a, b)
                        && sq_dist(normalized[a], normalized[b]) <= max2
                })
            })
            .collect();
        if kept.is_empty() {
            return Err(GeoError::GeometryInference(
                "triangulation retained no triangles at the requested max_link".into(),
            ));
        }

        let kept = largest_component(&kept);
        let (points, triangles) = compact(&kept, &d);
        let edge = boundary_rings(&triangles, &points)?;

        let mut out = TriSurface {
            points,
            triangles,
            edge,
            history: OperationHistory::new(),
        };
        *out.operation_history_mut() = self.operation_history().clone();
        out.record_history(format!("points.to_tri_surface(max_link={max_link})"));
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
/// When the walk labelled *both*, their `(column, row)` settles it: neighbouring grid
/// nodes differ by one in each index, and anything further apart is a link across a
/// hole or a fault — a link length cannot see that, because a one-cell fault throw is
/// metrically identical to a stretched cell (36 of the 84 bridges on the reference
/// surface are shorter than a legal quad diagonal). When either node went unlabelled,
/// we have no topology to appeal to and the length filter is all there is.
fn index_adjacent(d: &GridDetection, a: usize, b: usize) -> bool {
    match (d.index_of.get(&a), d.index_of.get(&b)) {
        (Some(&(ia, ja)), Some(&(ib, jb))) => (ia - ib).abs().max((ja - jb).abs()) == 1,
        _ => true,
    }
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

/// One surface, not several: keep the triangles of the largest connected component.
fn largest_component(tris: &[[usize; 3]]) -> Vec<[usize; 3]> {
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
    // Deterministic: on a tie, the smallest root wins (HashMap order is not stable).
    let best = size
        .iter()
        .max_by_key(|(&r, &n)| (n, std::cmp::Reverse(r)))
        .map(|(&r, _)| r);
    match best {
        Some(root) => tris
            .iter()
            .copied()
            .filter(|t| find(&mut parent, t[0]) == root)
            .collect(),
        None => Vec::new(),
    }
}

/// Reindex the surviving triangles onto only the vertices they use.
fn compact(tris: &[[usize; 3]], d: &GridDetection) -> (Vec<[f64; 3]>, Vec<[u32; 3]>) {
    let mut remap: HashMap<usize, u32> = HashMap::new();
    let mut points = Vec::new();
    for t in tris {
        for &v in t {
            remap.entry(v).or_insert_with(|| {
                points.push([d.pts[v][0], d.pts[v][1], d.zs[v]]);
                (points.len() - 1) as u32
            });
        }
    }
    let triangles = tris
        .iter()
        .map(|t| [remap[&t[0]], remap[&t[1]], remap[&t[2]]])
        .collect();
    (points, triangles)
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
            .to_tri_surface(None)
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
        let tin = PointSet::from_coords(coords).to_tri_surface(None).unwrap();
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

        let tin = p.to_tri_surface(None).unwrap();
        // One sheet, and no triangle spans the gap: every edge stays short in cells.
        assert_eq!(tin.edge().rings().len(), 1);
        assert!(
            tin.points().len() < 6 * 9 + 6 * 9,
            "the far block is dropped"
        );
        let xs: Vec<f64> = tin.points().iter().map(|p| p[0]).collect();
        let span = xs.iter().cloned().fold(f64::MIN, f64::max)
            - xs.iter().cloned().fold(f64::MAX, f64::min);
        assert!(
            span < 300.0,
            "the retained sheet must not cross the fault: {span}"
        );
    }

    #[test]
    fn is_deterministic() {
        // The boundary trace must not depend on hash iteration order. An undirected
        // trace has to guess which way to leave a pinch vertex, and the guess changed
        // the emitted rings from run to run on the same triangles.
        let coords = lattice(11, 9, 50.0, 30.0, 17.0);
        let p = PointSet::from_coords(coords);
        let first = p.to_tri_surface(None).unwrap();
        for _ in 0..8 {
            let again = p.to_tri_surface(None).unwrap();
            assert_eq!(again.triangles(), first.triangles());
            assert_eq!(again.points(), first.points());
            assert_eq!(again.edge().rings(), first.edge().rings());
        }
    }

    #[test]
    fn rejects_a_max_link_outside_the_band() {
        let p = PointSet::from_coords(lattice(6, 6, 50.0, 50.0, 0.0));
        for bad in [1.0, 1.41, 2.0, 2.5] {
            assert!(
                p.to_tri_surface(Some(bad)).is_err(),
                "max_link {bad} is outside (sqrt2, 2)"
            );
        }
        assert!(p.to_tri_surface(Some(1.8)).is_ok());
    }
}
