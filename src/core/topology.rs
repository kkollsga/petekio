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
    /// Detected step along the column axis. The two increments are resolved
    /// separately: a grid's cell need not be square.
    pub detected_cell_i: f64,
    /// Detected step along the row axis.
    pub detected_cell_j: f64,
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
    /// Fault blocks found. The walk re-seeds wherever it stalls, so one block means
    /// one uninterrupted grid; more than one means the surface is fault-cut.
    pub blocks: usize,
    /// Nodes in the biggest block. Most of the extra blocks on a real export are
    /// single snapped nodes along the clip boundary, not fault blocks.
    pub largest_block: usize,
}

impl TopologyReport {
    /// One fault block covering every distinct node, no index claimed twice, no
    /// unresolvable coincidence. Exactly the condition under which labels are
    /// returned: a structured mesh has a single `(column, row)` space, and a
    /// fault-cut surface has no such thing.
    pub fn verified(&self) -> bool {
        self.blocks == 1
            && self.assigned == self.distinct_nodes
            && self.conflicts == 0
            && self.coincident_ambiguous == 0
    }
}

/// Match radius around a predicted neighbour position, in units of the shorter step.
const R_PRED: f64 = 0.42;
/// A locally measured axial step is trusted only inside this band, relative to the
/// globally detected step for that axis.
const LOCAL_FRAME_BAND: (f64, f64) = (0.6, 1.6);
/// Radius, in units of the longer step, within which an unassigned point counts as
/// a stalled frontier candidate rather than simply absent.
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
        let d = self.detect_grid(nominal_cell)?;
        let report = d.report();
        if !report.verified() {
            return Ok((None, report));
        }
        Ok((Some(labelled(self, &d.pts, &d.zs, &d.index_of)), report))
    }

    /// The shared detection pass: dedupe, detect the axes and per-axis steps, walk.
    /// [`detect_topology`](Self::detect_topology) turns this into labels; the TIN
    /// fallback consumes its frontier as fault constraints.
    pub(crate) fn detect_grid(&self, nominal_cell: Option<f64>) -> Result<GridDetection> {
        if let Some(c) = nominal_cell {
            if !c.is_finite() || c <= 0.0 {
                return Err(GeoError::GeometryInference(
                    "nominal cell must be a finite positive number".into(),
                ));
            }
        }

        let (pts, zs, coincident_dropped, coincident_ambiguous) = distinct_nodes(self.coords())?;
        if pts.len() < 4 {
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

        let seed = detect_cell(&tree, &pts, nominal_cell)?;
        let azimuth = detect_azimuth(&tree, &pts, seed)?;
        let (e1, e2) = axes(azimuth);
        let (inc_i, inc_j) = detect_axis_steps(&tree, &pts, e1, e2, seed)?;
        let step_i = [e1[0] * inc_i, e1[1] * inc_i];
        let step_j = [e2[0] * inc_j, e2[1] * inc_j];

        let (index_of, conflicts, frontier, blocks) = walk(&tree, &pts, step_i, step_j);
        let mut block_size: HashMap<u32, usize> = HashMap::new();
        for &(b, ..) in index_of.values() {
            *block_size.entry(b).or_insert(0) += 1;
        }

        Ok(GridDetection {
            pts,
            zs,
            e1,
            e2,
            inc_i,
            inc_j,
            azimuth,
            index_of,
            conflicts,
            frontier,
            blocks,
            block_size,
            coincident_dropped,
            coincident_ambiguous,
        })
    }
}

/// The full result of one detection pass over a point set.
pub(crate) struct GridDetection {
    pub(crate) pts: Vec<[f64; 2]>,
    pub(crate) zs: Vec<f64>,
    pub(crate) e1: [f64; 2],
    pub(crate) e2: [f64; 2],
    pub(crate) inc_i: f64,
    pub(crate) inc_j: f64,
    azimuth: f64,
    pub(crate) index_of: HashMap<usize, NodeIndex>,
    conflicts: usize,
    /// Adjacencies the walk refused to resolve: each pair straddles a fault.
    pub(crate) frontier: Vec<(usize, usize)>,
    blocks: u32,
    /// Nodes per block.
    pub(crate) block_size: HashMap<u32, usize>,
    coincident_dropped: usize,
    coincident_ambiguous: usize,
}

impl GridDetection {
    pub(crate) fn report(&self) -> TopologyReport {
        TopologyReport {
            detected_cell_i: self.inc_i,
            detected_cell_j: self.inc_j,
            detected_azimuth_deg: self.azimuth.to_degrees(),
            distinct_nodes: self.pts.len(),
            assigned: self.index_of.len(),
            conflicts: self.conflicts,
            coincident_dropped: self.coincident_dropped,
            coincident_ambiguous: self.coincident_ambiguous,
            stalled_frontier: self.frontier.len(),
            blocks: self.blocks as usize,
            largest_block: self.block_size.values().copied().max().unwrap_or(0),
        }
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

    // Histogram every short neighbour step and take the modal bin. This is the
    // *seed* scale — the shorter of the two axes when the cell is anisotropic.
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

/// The step along each axis. A grid's two increments need not agree — a 50 x 25 m
/// Petrel cell is ordinary — so resolve them separately rather than assuming one
/// cell. `seed` is the shorter axis, from [`detect_cell`]; the longer one is found
/// within `AXIS_SEARCH` seeds of it.
fn detect_axis_steps(
    tree: &RTree<AerialEntry>,
    pts: &[[f64; 2]],
    e1: [f64; 2],
    e2: [f64; 2],
    seed: f64,
) -> Result<(f64, f64)> {
    const BINS: usize = 512;
    /// Candidates inspected per point. Must reach the long axis of a strongly
    /// anisotropic cell, whose neighbour sits many short-axis steps away.
    const CANDIDATES: usize = 64;
    /// How far out to look, in seeds — the seed is the *shorter* axis.
    const AXIS_SEARCH: f64 = 20.0;
    /// A neighbour vector counts as axial when it deviates less than this from the axis.
    const AXIAL_TAN: f64 = 0.30;

    let hi = AXIS_SEARCH * seed;
    // Histogram the *nearest* axial neighbour in each of the four directions. Taking
    // every axial neighbour instead would bin the step and all its multiples with
    // near-equal support, and the mode would pick an arbitrary multiple.
    let mut hist = [[0usize; BINS]; 2];
    let mut sums = [[0.0f64; BINS]; 2];

    for p in pts {
        let mut nearest = [[f64::INFINITY; 2]; 2]; // [axis][forward, backward]
        for e in tree.nearest_neighbor_iter(*p).take(CANDIDATES) {
            let q = e.geom();
            let v = [q[0] - p[0], q[1] - p[1]];
            let d = v[0].hypot(v[1]);
            if d <= 0.0 {
                continue;
            }
            if d > hi {
                break; // nearest_neighbor_iter yields in increasing distance
            }
            for (axis, (along_axis, across_axis)) in [(e1, e2), (e2, e1)].into_iter().enumerate() {
                let along = v[0] * along_axis[0] + v[1] * along_axis[1];
                let across = v[0] * across_axis[0] + v[1] * across_axis[1];
                if across.abs() > AXIAL_TAN * along.abs() {
                    continue;
                }
                let side = usize::from(along < 0.0);
                let mag = along.abs();
                if mag > 0.0 && mag < nearest[axis][side] {
                    nearest[axis][side] = mag;
                }
            }
            // Candidates arrive in increasing distance, so once every direction has a
            // nearest axial neighbour, nothing further can improve on it.
            if nearest.iter().flatten().all(|m| m.is_finite()) {
                break;
            }
        }
        for (axis, sides) in nearest.iter().enumerate() {
            for &mag in sides {
                if !mag.is_finite() {
                    continue;
                }
                let b = (((mag / hi) * BINS as f64) as usize).min(BINS - 1);
                hist[axis][b] += 1;
                sums[axis][b] += mag;
            }
        }
    }

    let mut out = [0.0f64; 2];
    for axis in 0..2 {
        let best = (0..BINS).max_by_key(|&b| hist[axis][b]).unwrap();
        if hist[axis][best] == 0 {
            return Err(GeoError::GeometryInference(
                "could not detect a step along both grid axes".into(),
            ));
        }
        out[axis] = sums[axis][best] / hist[axis][best] as f64;
    }
    Ok((out[0], out[1]))
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
/// A node's place in the grid: which fault block, and its `(column, row)` inside it.
pub(crate) type NodeIndex = (u32, i32, i32);

type WalkResult = (HashMap<usize, NodeIndex>, usize, Vec<(usize, usize)>, u32);

/// Label every node, one fault block at a time.
///
/// A single walk stops at the first fault: past a collapsed or stretched step the
/// neighbour relation is not determined by geometry. So re-seed in whatever is left
/// and walk again. Each block gets its own index origin, and a block boundary is
/// exactly a fault (or the surface's outer edge). Two nodes in different blocks have
/// no grid adjacency at all — which is what lets a triangulated fallback refuse to
/// bridge the fault it cannot otherwise see.
fn walk(
    tree: &RTree<AerialEntry>,
    pts: &[[f64; 2]],
    step_i: [f64; 2],
    step_j: [f64; 2],
) -> WalkResult {
    let mut index_of: HashMap<usize, NodeIndex> = HashMap::new();
    let mut conflicts = 0usize;
    let mut stalled: Vec<(usize, usize)> = Vec::new();
    let mut block = 0u32;

    while let Some(seed) = interior_seed(pts, &index_of) {
        walk_block(
            tree,
            pts,
            step_i,
            step_j,
            seed,
            block,
            &mut index_of,
            &mut conflicts,
            &mut stalled,
        );
        block += 1;
    }
    (index_of, conflicts, stalled, block)
}

#[allow(clippy::too_many_arguments)]
fn walk_block(
    tree: &RTree<AerialEntry>,
    pts: &[[f64; 2]],
    step_i: [f64; 2],
    step_j: [f64; 2],
    seed: usize,
    block: u32,
    index_of: &mut HashMap<usize, NodeIndex>,
    conflicts: &mut usize,
    stalled: &mut Vec<(usize, usize)>,
) {
    let mut owner: HashMap<(i32, i32), usize> = HashMap::new();
    index_of.insert(seed, (block, 0, 0));
    owner.insert((0, 0), seed);
    let mut queue = VecDeque::from([seed]);

    let len_i = step_i[0].hypot(step_i[1]);
    let len_j = step_j[0].hypot(step_j[1]);
    // The match radius must not exceed half the *shorter* step, or a prediction
    // could claim the wrong node on an anisotropic grid.
    let r2 = (R_PRED * len_i.min(len_j)).powi(2);
    let frontier_r2 = (FRONTIER_RADIUS * len_i.max(len_j)).powi(2);

    while let Some(k) = queue.pop_front() {
        let (_, i, j) = index_of[&k];
        let p = pts[k];

        // Predict with the *locally measured* step where labelled neighbours supply
        // one, so the walk follows the grid's curvature, shear and swell rather than
        // a single global frame.
        let li = local_step(&owner, pts, p, (i, j), (1, 0), len_i).unwrap_or(step_i);
        let lj = local_step(&owner, pts, p, (i, j), (0, 1), len_j).unwrap_or(step_j);

        for (vec, di, dj) in [
            (li, 1, 0),
            ([-li[0], -li[1]], -1, 0),
            (lj, 0, 1),
            ([-lj[0], -lj[1]], 0, -1),
        ] {
            let target = (i + di, j + dj);
            if owner.contains_key(&target) {
                continue;
            }
            let pred = [p[0] + vec[0], p[1] + vec[1]];

            let mut best: Option<(f64, usize)> = None;
            for e in tree.locate_within_distance(pred, r2) {
                let m = e.data;
                if let Some(&(owner_block, oi, oj)) = index_of.get(&m) {
                    // A node claimed by an EARLIER block is simply across a fault from
                    // here; that is the walk working, not a conflict. Only a clash
                    // inside this block's own index space is one.
                    if owner_block == block && (oi, oj) != target {
                        *conflicts += 1;
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
                    index_of.insert(m, (block, target.0, target.1));
                    owner.insert(target, m);
                    queue.push_back(m);
                }
                None => {
                    let len = vec[0].hypot(vec[1]);
                    let unit = [vec[0] / len, vec[1] / len];
                    if let Some(q) = frontier_candidate(tree, index_of, p, unit, len, frontier_r2) {
                        // The adjacency the walk could not resolve: `k` and `q` lie on
                        // opposite sides of a fault. Recording the pair lets a
                        // triangulated fallback refuse to bridge it.
                        stalled.push((k, q));
                    }
                }
            }
        }
    }
}

/// The locally measured step vector along one axis at `(i, j)`, averaged over
/// whichever of the two labelled neighbours along that axis exist. Returns `None`
/// when neither exists, or when the measured step falls outside the trusted band
/// around the global step — a stretched or collapsed step must not steer the walk.
fn local_step(
    owner: &HashMap<(i32, i32), usize>,
    pts: &[[f64; 2]],
    p: [f64; 2],
    (i, j): (i32, i32),
    (di, dj): (i32, i32),
    global_len: f64,
) -> Option<[f64; 2]> {
    let mut acc = [0.0f64; 2];
    let mut n = 0.0f64;
    for sign in [1i32, -1] {
        let key = (i + di * sign, j + dj * sign);
        if let Some(&m) = owner.get(&key) {
            acc[0] += sign as f64 * (pts[m][0] - p[0]);
            acc[1] += sign as f64 * (pts[m][1] - p[1]);
            n += 1.0;
        }
    }
    if n == 0.0 {
        return None;
    }
    let v = [acc[0] / n, acc[1] / n];
    let len = v[0].hypot(v[1]);
    if len <= LOCAL_FRAME_BAND.0 * global_len || len >= LOCAL_FRAME_BAND.1 * global_len {
        return None;
    }
    Some(v)
}

/// Is there an unlabelled point out along `vec` that we failed to claim? Then the
/// walk stalled here — a fault trace — rather than simply running off the edge.
fn frontier_candidate(
    tree: &RTree<AerialEntry>,
    index_of: &HashMap<usize, NodeIndex>,
    p: [f64; 2],
    unit: [f64; 2],
    step_len: f64,
    frontier_r2: f64,
) -> Option<usize> {
    let mut best: Option<(f64, usize)> = None;
    for e in tree.locate_within_distance(p, frontier_r2) {
        let m = e.data;
        if index_of.contains_key(&m) {
            continue;
        }
        let q = e.geom();
        let (dx, dy) = (q[0] - p[0], q[1] - p[1]);
        let d = dx.hypot(dy);
        if d < 0.5 * step_len {
            continue;
        }
        if (dx * unit[0] + dy * unit[1]) / d > FRONTIER_COS && best.is_none_or(|b| d < b.0) {
            best = Some((d, m));
        }
    }
    best.map(|(_, m)| m)
}

/// A deep-interior seed among the still-unlabelled nodes: the one closest to their
/// centroid. Boundary nodes are exactly the snapped ones, so seeding there strands the
/// walk immediately. Returns `None` once every node carries a label.
fn interior_seed(pts: &[[f64; 2]], index_of: &HashMap<usize, NodeIndex>) -> Option<usize> {
    let free: Vec<usize> = (0..pts.len())
        .filter(|k| !index_of.contains_key(k))
        .collect();
    if free.is_empty() {
        return None;
    }
    let n = free.len() as f64;
    let cx = free.iter().map(|&k| pts[k][0]).sum::<f64>() / n;
    let cy = free.iter().map(|&k| pts[k][1]).sum::<f64>() / n;
    let mut best = (f64::INFINITY, free[0]);
    for &k in &free {
        let d = (pts[k][0] - cx).powi(2) + (pts[k][1] - cy).powi(2);
        if d < best.0 {
            best = (d, k);
        }
    }
    Some(best.1)
}

/// Emit the labelled point set: original coordinates, plus 1-based `column`/`row`.
fn labelled(
    src: &PointSet,
    pts: &[[f64; 2]],
    zs: &[f64],
    index_of: &HashMap<usize, NodeIndex>,
) -> PointSet {
    // Only ever called on a verified (single-block) detection.
    let min_i = index_of.values().map(|v| v.1).min().unwrap();
    let min_j = index_of.values().map(|v| v.2).min().unwrap();

    let mut order: Vec<(&usize, &NodeIndex)> = index_of.iter().collect();
    order.sort_by_key(|(_, &(_, i, j))| (j, i));

    let mut coords = Vec::with_capacity(order.len());
    let mut columns = Vec::with_capacity(order.len());
    let mut rows = Vec::with_capacity(order.len());
    for (&k, &(_, i, j)) in order {
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
        assert_eq!(report.blocks, 1, "an unfaulted grid is one block");
        assert_eq!(report.largest_block, coords.len());
        assert_eq!(report.conflicts, 0);
        // This fixture deliberately swells along i, so the modal i-step is a little
        // above the nominal 50 m. That is the grid, not an estimator error.
        assert!(
            (50.0..54.0).contains(&report.detected_cell_i),
            "i-step {} outside the swelling grid's range",
            report.detected_cell_i
        );
        approx::assert_relative_eq!(report.detected_cell_j, 50.0, epsilon = 1.5);

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
        // The walk re-seeds where it stalls, so every node is labelled -- but in more
        // than one block, and a structured mesh has only one (column, row) space.
        assert_eq!(report.assigned, report.distinct_nodes);
        assert!(
            report.blocks >= 2,
            "the fault must split the grid into blocks: {report:?}"
        );
        // NB: stalled_frontier can be zero when the throw exceeds the frontier search
        // radius; the block split is the reliable signal, not the frontier count.
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

    /// A rotated, anisotropic, sheared lattice — the shape a real Petrel grid takes.
    fn anisotropic(
        ncol: usize,
        nrow: usize,
        az_deg: f64,
        xinc: f64,
        yinc: f64,
        shear: f64,
    ) -> Vec<[f64; 3]> {
        let (s, c) = az_deg.to_radians().sin_cos();
        let mut out = Vec::new();
        for j in 0..nrow {
            for i in 0..ncol {
                let (fi, fj) = (i as f64, j as f64);
                let (u, v) = (xinc * fi + shear * fj, yinc * fj);
                out.push([
                    1000.0 + u * c - v * s,
                    2000.0 + u * s + v * c,
                    -1800.0 - fi - fj,
                ]);
            }
        }
        out
    }

    #[test]
    fn detects_anisotropic_cells() {
        // A grid's two increments need not agree. Resolving one "cell" and predicting
        // both axes with it strands the walk the moment the cell is not square.
        for (xinc, yinc) in [(50.0, 50.0), (50.0, 25.0), (25.0, 50.0), (20.0, 200.0)] {
            let p = PointSet::from_coords(anisotropic(12, 10, 30.0, xinc, yinc, 0.0));
            let (labelled, report) = p.detect_topology(None).unwrap();
            assert!(
                report.verified(),
                "an anisotropic {xinc}x{yinc} grid must verify: {report:?}"
            );
            assert!(labelled.is_some());
            let (lo, hi) = (xinc.min(yinc), xinc.max(yinc));
            let (di, dj) = (report.detected_cell_i, report.detected_cell_j);
            approx::assert_relative_eq!(di.min(dj), lo, epsilon = 1e-6);
            approx::assert_relative_eq!(di.max(dj), hi, epsilon = 1e-6);
        }
    }

    #[test]
    fn detects_a_sheared_anisotropic_grid() {
        let p = PointSet::from_coords(anisotropic(14, 12, 37.0, 50.0, 25.0, 4.0));
        let (labelled, report) = p.detect_topology(None).unwrap();
        assert!(
            report.verified(),
            "shear must not derail the walk: {report:?}"
        );
        assert_eq!(report.assigned, 14 * 12);
        assert!(labelled.is_some());
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
