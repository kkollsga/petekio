//! Grid topology recovery from unlabelled surface points.
//!
//! A gridded surface exported as bare `X Y Z` has lost its `(column, row)`
//! topology, and the points do not in general lie on any regular lattice —
//! Petrel snaps nodes onto fault traces and onto the clip boundary, and the
//! grid lines themselves curve. **The points are the ground truth**: this
//! module never moves one. It detects the grid's rotation and cell size, walks
//! the grid paths to *label* each point, and then either verifies the result or
//! refuses.
//!
//! Refusing is the point. The walk cannot cross a fault — where nodes collapse
//! onto the fault trace or stretch across it, the neighbour relation simply is
//! not determined by geometry, and forcing it silently welds fault blocks
//! together. The stall is therefore a *fault detector*, and a caller that gets
//! an unverified report should fall back to a triangulated surface.
//!
//! Spec: `surface_topology_walk_spec` on the planning graph.

use crate::core::points::AerialEntry;
use crate::core::PointSet;
use crate::foundation::{GeoError, HasHistory, Result};
use indexmap::IndexMap;
use rstar::primitives::GeomWithData;
use rstar::RTree;
use std::collections::{HashMap, VecDeque};

/// What [`PointSet::detect_topology`] learned about a point set.
///
/// [`verified`](Self::verified) is the gate: only a verified detection may be
/// promoted to a structured mesh.
#[derive(Debug, Clone, PartialEq)]
pub struct TopologyReport {
    /// Detected cell size, from the modal nearest-neighbour step.
    pub detected_cell: f64,
    /// Detected grid azimuth in degrees, modulo 90 (the axes are interchangeable).
    pub detected_azimuth_deg: f64,
    /// Distinct nodes considered (after dropping exactly-coincident duplicates).
    pub distinct_nodes: usize,
    /// Nodes the walk reached and labelled.
    pub assigned: usize,
    /// Times two points claimed the same `(column, row)`.
    pub conflicts: usize,
    /// Coincident points dropped: same XY *and* same Z, so the index each takes
    /// cannot change the surface.
    pub coincident_dropped: usize,
    /// Coincident points that could *not* be dropped: same XY, different Z. Two
    /// distinct nodes at one location, and nothing decides which is which.
    pub coincident_ambiguous: usize,
    /// Adjacencies the walk could not resolve although a candidate lay in that
    /// direction — the fault traces, in point-index form.
    pub stalled_frontier: usize,
}

impl TopologyReport {
    /// Every distinct node labelled, no index claimed twice, no unresolvable
    /// coincidence. Exactly the condition under which labels are returned.
    pub fn verified(&self) -> bool {
        self.assigned == self.distinct_nodes
            && self.conflicts == 0
            && self.coincident_ambiguous == 0
    }
}

/// Match radius around a predicted neighbour position, in cells.
const R_PRED: f64 = 0.42;
/// A re-estimated local axial step is trusted only inside this band, in cells.
const LOCAL_FRAME_BAND: (f64, f64) = (0.6, 1.6);
/// Radius, in cells, within which an unassigned point counts as a stalled
/// frontier candidate rather than simply absent.
const FRONTIER_RADIUS: f64 = 2.2;
/// Angular tolerance for the frontier test.
const FRONTIER_COS: f64 = 0.951; // cos(18 degrees)

impl PointSet {
    /// Recover `(column, row)` topology from bare coordinates, without moving a point.
    ///
    /// Returns the labelled points **only when the detection verifies** — every
    /// distinct node reached, no index claimed twice. Otherwise the points are
    /// `None` and the report says why; the caller should then represent the
    /// surface as a triangulated network rather than a structured mesh.
    ///
    /// `nominal_cell` seeds the scale search; when `None` it is taken from the
    /// median nearest-neighbour distance.
    ///
    /// Coincident points (same XY and same Z) are dropped: a fault-collapsed node
    /// pair is geometrically identical, so which index each takes cannot change
    /// the surface. Coincident points with *differing* Z are a genuine ambiguity
    /// and fail the detection.
    pub fn detect_topology(
        &self,
        nominal_cell: Option<f64>,
    ) -> Result<(Option<PointSet>, TopologyReport)> {
        if let Some(c) = nominal_cell {
            if !c.is_finite() || c <= 0.0 {
                return Err(GeoError::GeometryInference(
                    "nominal cell must be a finite positive number".into(),
                ));
            }
        }

        let (pts, zs, coincident_dropped, coincident_ambiguous) = distinct_nodes(self.coords())?;
        let n = pts.len();
        if n < 4 {
            return Err(GeoError::GeometryInference(
                "topology detection requires at least four distinct finite points".into(),
            ));
        }

        let entries: Vec<AerialEntry> = pts
            .iter()
            .enumerate()
            .map(|(i, p)| GeomWithData::new(*p, i))
            .collect();
        let tree = RTree::bulk_load(entries);

        let cell = detect_cell(&tree, &pts, nominal_cell)?;
        let azimuth = detect_azimuth(&tree, &pts, cell)?;
        let (e1, e2) = axes(azimuth);

        let (index_of, conflicts, stalled) = walk(&tree, &pts, cell, e1, e2);

        let report = TopologyReport {
            detected_cell: cell,
            detected_azimuth_deg: azimuth.to_degrees(),
            distinct_nodes: n,
            assigned: index_of.len(),
            conflicts,
            coincident_dropped,
            coincident_ambiguous,
            stalled_frontier: stalled,
        };

        if !report.verified() {
            return Ok((None, report));
        }

        Ok((Some(labelled(self, &pts, &zs, &index_of)), report))
    }
}

/// Deduplicate exactly-coincident points. Returns the distinct XY, their Z, how
/// many identical duplicates were dropped, and how many XY carried two different
/// Z (each an unresolvable ambiguity).
type DistinctNodes = (Vec<[f64; 2]>, Vec<f64>, usize, usize);
fn distinct_nodes(coords: &[[f64; 3]]) -> Result<DistinctNodes> {
    let mut seen: HashMap<(u64, u64), f64> = HashMap::new();
    let mut pts = Vec::new();
    let mut zs = Vec::new();
    let mut dropped = 0usize;
    let mut ambiguous = 0usize;
    for c in coords {
        if !c[0].is_finite() || !c[1].is_finite() {
            continue;
        }
        let key = (c[0].to_bits(), c[1].to_bits());
        match seen.get(&key) {
            Some(z) => {
                if z.to_bits() == c[2].to_bits() {
                    dropped += 1;
                } else {
                    ambiguous += 1;
                }
            }
            None => {
                seen.insert(key, c[2]);
                pts.push([c[0], c[1]]);
                zs.push(c[2]);
            }
        }
    }
    Ok((pts, zs, dropped, ambiguous))
}

/// Cell size from the modal nearest-neighbour step. The mode, not the mean:
/// collapsed and stretched steps are outliers, and a mean is dragged by them.
fn detect_cell(tree: &RTree<AerialEntry>, pts: &[[f64; 2]], hint: Option<f64>) -> Result<f64> {
    let seed = match hint {
        Some(c) => c,
        None => {
            let mut first: Vec<f64> = pts
                .iter()
                .filter_map(|p| {
                    tree.nearest_neighbor_iter(*p)
                        .find(|e| dist(e.geom(), p) > 0.0)
                        .map(|e| dist(e.geom(), p))
                })
                .collect();
            if first.is_empty() {
                return Err(GeoError::GeometryInference(
                    "no neighbouring points to detect a cell size".into(),
                ));
            }
            first.sort_by(f64::total_cmp);
            first[first.len() / 2]
        }
    };
    if !seed.is_finite() || seed <= 0.0 {
        return Err(GeoError::GeometryInference(
            "could not detect a positive cell size".into(),
        ));
    }

    // Histogram every short neighbour step and take the modal bin.
    let hi = 1.4 * seed;
    const BINS: usize = 240;
    let mut hist = [0usize; BINS];
    let mut sums = [0.0f64; BINS];
    for p in pts {
        for e in tree.nearest_neighbor_iter(*p).take(6) {
            let d = dist(e.geom(), p);
            if d <= 0.0 || d > hi {
                continue;
            }
            let b = ((d / hi) * BINS as f64) as usize;
            let b = b.min(BINS - 1);
            hist[b] += 1;
            sums[b] += d;
        }
    }
    let best = (0..BINS).max_by_key(|&b| hist[b]).unwrap();
    if hist[best] == 0 {
        return Err(GeoError::GeometryInference(
            "could not detect a cell size from neighbour steps".into(),
        ));
    }
    Ok(sums[best] / hist[best] as f64)
}

/// Grid azimuth, modulo 90 degrees. Averaged on the quadrupled angle so the
/// four-fold ambiguity of a square lattice does not fight the circular mean.
fn detect_azimuth(tree: &RTree<AerialEntry>, pts: &[[f64; 2]], cell: f64) -> Result<f64> {
    let (lo, hi) = (0.7 * cell, 1.3 * cell);
    let (mut sx, mut sy) = (0.0f64, 0.0f64);
    let mut count = 0usize;
    for p in pts {
        for e in tree.nearest_neighbor_iter(*p).take(5) {
            let q = e.geom();
            let d = dist(q, p);
            if d < lo || d > hi {
                continue;
            }
            let t = 4.0 * (q[1] - p[1]).atan2(q[0] - p[0]);
            sx += t.cos();
            sy += t.sin();
            count += 1;
        }
    }
    if count == 0 {
        return Err(GeoError::GeometryInference(
            "could not detect a grid azimuth: no neighbour steps near the detected cell".into(),
        ));
    }
    Ok(sy.atan2(sx) / 4.0)
}

fn axes(azimuth: f64) -> ([f64; 2], [f64; 2]) {
    let (s, c) = azimuth.sin_cos();
    ([c, s], [-s, c])
}

fn dist(a: &[f64; 2], b: &[f64; 2]) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

/// Breadth-first predictive walk. Each labelled node predicts its four axial
/// neighbours and claims the nearest unlabelled point near that prediction.
/// Unmatched directions leave a hole, which is legal — a surface need not fill
/// its lattice. Returns the labels, the conflict count, and the stalled frontier.
fn walk(
    tree: &RTree<AerialEntry>,
    pts: &[[f64; 2]],
    cell: f64,
    e1: [f64; 2],
    e2: [f64; 2],
) -> (HashMap<usize, (i32, i32)>, usize, usize) {
    let seed = interior_seed(pts);
    let mut index_of: HashMap<usize, (i32, i32)> = HashMap::new();
    let mut owner: HashMap<(i32, i32), usize> = HashMap::new();
    index_of.insert(seed, (0, 0));
    owner.insert((0, 0), seed);
    let mut queue = VecDeque::from([seed]);

    let r_pred = R_PRED * cell;
    let r2 = r_pred * r_pred;
    let frontier_r2 = (FRONTIER_RADIUS * cell).powi(2);
    let mut conflicts = 0usize;
    let mut stalled = 0usize;

    while let Some(k) = queue.pop_front() {
        let (i, j) = index_of[&k];
        let p = pts[k];

        // Re-estimate the local frame from already-labelled axial neighbours so the
        // walk follows the grid's curvature instead of a single global azimuth.
        let (le1, le2) = local_frame(&owner, pts, i, j, p, cell).unwrap_or((e1, e2));

        for (vec, di, dj) in [
            (le1, 1, 0),
            ([-le1[0], -le1[1]], -1, 0),
            (le2, 0, 1),
            ([-le2[0], -le2[1]], 0, -1),
        ] {
            let target = (i + di, j + dj);
            if owner.contains_key(&target) {
                continue;
            }
            let pred = [p[0] + vec[0] * cell, p[1] + vec[1] * cell];

            let mut best: Option<(f64, usize)> = None;
            for e in tree.locate_within_distance(pred, r2) {
                let m = e.data;
                if index_of.contains_key(&m) {
                    if index_of[&m] != target {
                        // Someone else already owns this point at a different index.
                        conflicts += 1;
                    }
                    continue;
                }
                let d = dist(e.geom(), &pred);
                if best.is_none() || d < best.unwrap().0 {
                    best = Some((d, m));
                }
            }

            match best {
                Some((_, m)) => {
                    index_of.insert(m, target);
                    owner.insert(target, m);
                    queue.push_back(m);
                }
                None => {
                    if frontier_candidate(tree, &index_of, p, vec, cell, frontier_r2) {
                        stalled += 1;
                    }
                }
            }
        }
    }
    (index_of, conflicts, stalled)
}

/// The local axial frame at `(i, j)`, from labelled neighbours along +/- i.
fn local_frame(
    owner: &HashMap<(i32, i32), usize>,
    pts: &[[f64; 2]],
    i: i32,
    j: i32,
    p: [f64; 2],
    cell: f64,
) -> Option<([f64; 2], [f64; 2])> {
    let mut acc = [0.0f64; 2];
    let mut n = 0.0f64;
    for (di, sign) in [(1i32, 1.0f64), (-1, -1.0)] {
        if let Some(&m) = owner.get(&(i + di, j)) {
            acc[0] += sign * (pts[m][0] - p[0]);
            acc[1] += sign * (pts[m][1] - p[1]);
            n += 1.0;
        }
    }
    if n == 0.0 {
        return None;
    }
    let v = [acc[0] / n, acc[1] / n];
    let len = v[0].hypot(v[1]);
    if len <= LOCAL_FRAME_BAND.0 * cell || len >= LOCAL_FRAME_BAND.1 * cell {
        return None;
    }
    let e1 = [v[0] / len, v[1] / len];
    Some((e1, [-e1[1], e1[0]]))
}

/// Is there an unlabelled point out along `vec` that we failed to claim? Then the
/// walk stalled here — a fault trace — rather than simply running off the edge.
fn frontier_candidate(
    tree: &RTree<AerialEntry>,
    index_of: &HashMap<usize, (i32, i32)>,
    p: [f64; 2],
    vec: [f64; 2],
    cell: f64,
    frontier_r2: f64,
) -> bool {
    for e in tree.locate_within_distance(p, frontier_r2) {
        let m = e.data;
        if index_of.contains_key(&m) {
            continue;
        }
        let q = e.geom();
        let (dx, dy) = (q[0] - p[0], q[1] - p[1]);
        let d = dx.hypot(dy);
        if d < 0.5 * cell {
            continue;
        }
        if (dx * vec[0] + dy * vec[1]) / d > FRONTIER_COS {
            return true;
        }
    }
    false
}

/// A deep-interior seed: the point closest to the centroid. Boundary nodes are
/// exactly the snapped ones, so seeding there strands the walk immediately.
fn interior_seed(pts: &[[f64; 2]]) -> usize {
    let n = pts.len() as f64;
    let cx = pts.iter().map(|p| p[0]).sum::<f64>() / n;
    let cy = pts.iter().map(|p| p[1]).sum::<f64>() / n;
    let mut best = (f64::INFINITY, 0usize);
    for (k, p) in pts.iter().enumerate() {
        let d = (p[0] - cx).powi(2) + (p[1] - cy).powi(2);
        if d < best.0 {
            best = (d, k);
        }
    }
    best.1
}

/// Emit the labelled point set: original coordinates, plus 1-based `column`/`row`.
fn labelled(
    src: &PointSet,
    pts: &[[f64; 2]],
    zs: &[f64],
    index_of: &HashMap<usize, (i32, i32)>,
) -> PointSet {
    let min_i = index_of.values().map(|v| v.0).min().unwrap();
    let min_j = index_of.values().map(|v| v.1).min().unwrap();

    let mut order: Vec<(&usize, &(i32, i32))> = index_of.iter().collect();
    order.sort_by_key(|(_, &(i, j))| (j, i));

    let mut coords = Vec::with_capacity(order.len());
    let mut columns = Vec::with_capacity(order.len());
    let mut rows = Vec::with_capacity(order.len());
    for (&k, &(i, j)) in order {
        coords.push([pts[k][0], pts[k][1], zs[k]]);
        columns.push((i - min_i + 1) as f64);
        rows.push((j - min_j + 1) as f64);
    }
    let mut attrs = IndexMap::new();
    attrs.insert("column".to_string(), columns);
    attrs.insert("row".to_string(), rows);
    let mut out = PointSet::from_parts(coords, attrs);
    *out.operation_history_mut() = src.operation_history().clone();
    out.record_history("points.detect_topology()");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::GeometryEdge;

    /// A curvilinear grid with an unpopulated corner: cell size swells, the rows
    /// curve, and the footprint is not rectangular. The walk must still label it.
    fn curvilinear(ncol: usize, nrow: usize) -> Vec<[f64; 3]> {
        let mut out = Vec::new();
        for j in 0..nrow {
            for i in 0..ncol {
                if i >= ncol - 2 && j >= nrow - 2 {
                    continue;
                }
                let (x, y) = node(i, j);
                out.push([x, y, -1800.0 - (i + j) as f64]);
            }
        }
        out
    }

    fn node(i: usize, j: usize) -> (f64, f64) {
        let (i, j) = (i as f64, j as f64);
        // ~50 m cell, rotated ~30 degrees, with a gentle swell and bow.
        let u = 50.0 * i * (1.0 + 0.004 * i);
        let v = 50.0 * j + 0.35 * i * i * 0.02;
        let (s, c) = 30f64.to_radians().sin_cos();
        (1000.0 + u * c - v * s, 2000.0 + u * s + v * c)
    }

    #[test]
    fn detects_topology_of_a_curvilinear_grid_and_round_trips_exactly() {
        let coords = curvilinear(9, 7);
        let p = PointSet::from_coords(coords.clone());

        let (labelled, report) = p.detect_topology(None).unwrap();
        assert!(
            report.verified(),
            "walk must label every node of an unfaulted grid: {report:?}"
        );
        assert_eq!(report.assigned, coords.len());
        assert_eq!(report.conflicts, 0);
        approx::assert_relative_eq!(report.detected_cell, 50.0, epsilon = 1.5);

        let labelled = labelled.expect("verified detection yields labelled points");
        assert_eq!(labelled.len(), coords.len());

        // The whole point: the labels let the mesh be built, and nothing moved.
        let mesh = labelled
            .to_structured_surface(1e-3, GeometryEdge::Occupied)
            .expect("labelled points form a structured mesh");
        let back = mesh.to_points();
        assert_eq!(back.len(), coords.len());

        let mut before: Vec<[u64; 3]> = coords
            .iter()
            .map(|c| [c[0].to_bits(), c[1].to_bits(), c[2].to_bits()])
            .collect();
        let mut after: Vec<[u64; 3]> = back
            .coords()
            .iter()
            .map(|c| [c[0].to_bits(), c[1].to_bits(), c[2].to_bits()])
            .collect();
        before.sort();
        after.sort();
        assert_eq!(
            before, after,
            "points -> topology -> mesh -> points is exact"
        );
    }

    #[test]
    fn refuses_to_walk_across_a_fault() {
        // Two blocks of the same grid, offset along strike by half a cell and pulled
        // apart by more than a cell: the neighbour relation across the gap is not
        // determined by geometry, so the walk must stall rather than guess.
        let mut coords = Vec::new();
        for j in 0..8 {
            for i in 0..6 {
                let (x, y) = node(i, j);
                coords.push([x, y, -1800.0]);
            }
        }
        for j in 0..8 {
            for i in 8..14 {
                let (x, y) = node(i, j);
                coords.push([x + 30.0, y + 25.0, -1900.0]);
            }
        }
        let p = PointSet::from_coords(coords);
        let (labelled, report) = p.detect_topology(None).unwrap();

        assert!(!report.verified(), "a fault-cut grid must not verify");
        assert!(labelled.is_none(), "unverified detection yields no labels");
        assert!(
            report.assigned < report.distinct_nodes,
            "the walk must stall at the fault, leaving nodes unlabelled: {report:?}"
        );
    }

    #[test]
    fn drops_identical_coincident_nodes_but_refuses_ambiguous_ones() {
        // Same XY, same Z: a fault-collapsed pair. Which index it takes cannot change
        // the surface, so drop one and carry on.
        let mut coords = curvilinear(7, 6);
        let dup = coords[10];
        coords.push(dup);
        let p = PointSet::from_coords(coords.clone());
        let (labelled, report) = p.detect_topology(None).unwrap();
        assert_eq!(report.coincident_dropped, 1);
        assert!(
            report.verified(),
            "an identical duplicate is harmless: {report:?}"
        );
        assert!(labelled.is_some());

        // Same XY, different Z: two genuinely distinct nodes at one location. Refuse.
        let mut coords = curvilinear(7, 6);
        let mut dup = coords[10];
        dup[2] += 25.0;
        coords.push(dup);
        let p = PointSet::from_coords(coords);
        let (labelled, report) = p.detect_topology(None).unwrap();
        assert_eq!(report.coincident_ambiguous, 1);
        assert_eq!(report.coincident_dropped, 0);
        assert!(
            labelled.is_none(),
            "coincident XY with differing Z is two nodes at one place: refuse"
        );
        assert!(
            !report.verified(),
            "verified() must agree with whether labels were returned: {report:?}"
        );
    }

    #[test]
    fn rejects_degenerate_input() {
        let p = PointSet::from_coords(vec![[0.0, 0.0, 1.0], [1.0, 0.0, 1.0]]);
        assert!(p.detect_topology(None).is_err());

        let p = PointSet::from_coords(curvilinear(5, 5));
        assert!(p.detect_topology(Some(-1.0)).is_err());
        assert!(p.detect_topology(Some(f64::NAN)).is_err());
    }
}
