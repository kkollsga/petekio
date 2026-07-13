//! `algorithms::surfaces` — pure surface-numeric kernels: iso-line
//! (contour) extraction over a triangle mesh with per-node values.
//!
//! Type-light by the constitution: primitives in (`nodes`/`triangles`/`values`
//! slices), primitives out. Domain types (`Surface`, `StructuredMeshSurface`,
//! `TriSurface`) call in through their shells; the marching-triangles math has
//! this one home.

use crate::foundation::{GeoError, Result};
use ndarray::Array2;
use std::collections::{BTreeMap, BTreeSet};

/// Dip angle and down-dip azimuth for a regular lattice's value field.
///
/// Derivatives use central differences where both neighbours are defined and
/// one-sided differences at boundaries or beside holes. `x_step` is the signed
/// world distance along the lattice I axis; `y_step` is the signed distance
/// along J (therefore includes y-flip). The local gradient is rotated into
/// world East/North before the conventional geological outputs are calculated.
/// Source holes and nodes lacking a derivative on either axis stay `NaN`.
pub fn dip_fields(
    values: &Array2<f64>,
    x_step: f64,
    y_step: f64,
    rotation_deg: f64,
) -> (Array2<f64>, Array2<f64>) {
    let (ncol, nrow) = values.dim();
    let mut angle = Array2::from_elem((ncol, nrow), f64::NAN);
    let mut azimuth = Array2::from_elem((ncol, nrow), f64::NAN);
    let (sin_theta, cos_theta) = rotation_deg.to_radians().sin_cos();

    for j in 0..nrow {
        for i in 0..ncol {
            if values[[i, j]].is_nan() {
                continue;
            }
            let Some(du) = axis_derivative(values, i, j, true, x_step) else {
                continue;
            };
            let Some(dv) = axis_derivative(values, i, j, false, y_step) else {
                continue;
            };
            let gx = du * cos_theta - dv * sin_theta;
            let gy = du * sin_theta + dv * cos_theta;
            let slope = gx.hypot(gy);
            angle[[i, j]] = slope.atan().to_degrees();
            if slope != 0.0 {
                azimuth[[i, j]] = (-gx).atan2(-gy).to_degrees().rem_euclid(360.0);
            }
        }
    }
    (angle, azimuth)
}

fn axis_derivative(
    values: &Array2<f64>,
    i: usize,
    j: usize,
    along_i: bool,
    step: f64,
) -> Option<f64> {
    if step == 0.0 || !step.is_finite() {
        return None;
    }
    let (ncol, nrow) = values.dim();
    let centre = values[[i, j]];
    let minus = if along_i {
        i.checked_sub(1).map(|ii| values[[ii, j]])
    } else {
        j.checked_sub(1).map(|jj| values[[i, jj]])
    }
    .filter(|v| !v.is_nan());
    let plus = if along_i {
        (i + 1 < ncol).then(|| values[[i + 1, j]])
    } else {
        (j + 1 < nrow).then(|| values[[i, j + 1]])
    }
    .filter(|v| !v.is_nan());

    match (minus, plus) {
        (Some(lo), Some(hi)) => Some((hi - lo) / (2.0 * step)),
        (None, Some(hi)) => Some((hi - centre) / step),
        (Some(lo), None) => Some((centre - lo) / step),
        (None, None) => None,
    }
}

/// Iso-levels aligned to multiples of `interval` spanning `[vmin, vmax]`.
/// Empty when the range is empty/non-finite. Errors on a non-positive interval.
pub fn aligned_levels(vmin: f64, vmax: f64, interval: f64) -> Result<Vec<f64>> {
    if !interval.is_finite() || interval <= 0.0 {
        return Err(GeoError::OutOfRange(format!(
            "iso-line interval must be a finite positive number, got {interval}"
        )));
    }
    if !vmin.is_finite() || !vmax.is_finite() || vmin > vmax {
        return Ok(Vec::new());
    }
    // A hair of slack so a range endpoint that *is* a multiple stays included.
    let eps = 1e-9 * interval.max(vmax.abs()).max(vmin.abs());
    let k0 = ((vmin - eps) / interval).ceil() as i64;
    let k1 = ((vmax + eps) / interval).floor() as i64;
    Ok((k0..=k1).map(|k| k as f64 * interval).collect())
}

/// Douglas–Peucker polyline simplification with a world-unit tolerance.
///
/// Drops vertices that lie within `tol` of the retained chords, preserving the
/// line's shape (a corner sitting farther than `tol` from its chord is always
/// kept). Type-light by the constitution: `[x, y]` vertices in, a simplified
/// vertex list out; the domain layers call in.
///
/// Guarantees:
/// * The endpoints of an **open** line are preserved; it never simplifies below
///   two points.
/// * A **closed ring** (first vertex equals last) keeps its closure and never
///   simplifies below four points (three distinct corners + the repeat) — so a
///   ring stays a ring at every tolerance.
/// * `tol <= 0` (or a non-finite tol) is a no-op: the input is returned as-is.
pub fn douglas_peucker(points: &[[f64; 2]], tol: f64) -> Vec<[f64; 2]> {
    let n = points.len();
    if n <= 2 || !tol.is_finite() || tol <= 0.0 {
        return points.to_vec();
    }
    let closed = points[0] == points[n - 1];
    if !closed {
        return dp_open(points, tol);
    }
    // A ring's chord (first == last) is degenerate, so split it at the vertex
    // farthest from the anchor into two open chains and simplify each.
    if n <= 4 {
        return points.to_vec(); // already a minimal ring
    }
    let anchor = points[0];
    let (mut split, mut best) = (1usize, f64::NEG_INFINITY);
    for (k, p) in points.iter().enumerate().take(n - 1).skip(1) {
        let d = sq_dist(*p, anchor);
        if d > best {
            best = d;
            split = k;
        }
    }
    let mut head = dp_open(&points[..=split], tol);
    let tail = dp_open(&points[split..], tol);
    head.pop(); // the split vertex is shared by both chains
    head.extend(tail);
    if head.len() < 4 {
        return points.to_vec(); // never collapse a ring below four points
    }
    head
}

/// Douglas–Peucker on an open polyline: endpoints fixed, recursion keeps any
/// vertex farther than `tol` from the current chord. Never returns < 2 points.
fn dp_open(points: &[[f64; 2]], tol: f64) -> Vec<[f64; 2]> {
    let n = points.len();
    if n <= 2 {
        return points.to_vec();
    }
    let mut keep = vec![false; n];
    keep[0] = true;
    keep[n - 1] = true;
    dp_recurse(points, 0, n - 1, tol, &mut keep);
    points
        .iter()
        .zip(&keep)
        .filter_map(|(p, &k)| if k { Some(*p) } else { None })
        .collect()
}

fn dp_recurse(points: &[[f64; 2]], lo: usize, hi: usize, tol: f64, keep: &mut [bool]) {
    if hi <= lo + 1 {
        return;
    }
    let (a, b) = (points[lo], points[hi]);
    let (mut far, mut best) = (lo, -1.0);
    for (k, p) in points.iter().enumerate().take(hi).skip(lo + 1) {
        let d = point_line_dist(*p, a, b);
        if d > best {
            best = d;
            far = k;
        }
    }
    if best > tol {
        keep[far] = true;
        dp_recurse(points, lo, far, tol, keep);
        dp_recurse(points, far, hi, tol, keep);
    }
}

/// Perpendicular distance from `p` to the line through `a`–`b` (to the point
/// `a` when the chord is degenerate).
fn point_line_dist(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let len2 = dx * dx + dy * dy;
    if len2 <= 0.0 {
        return sq_dist(p, a).sqrt();
    }
    // |cross(b-a, p-a)| / |b-a|
    ((p[0] - a[0]) * dy - (p[1] - a[1]) * dx).abs() / len2.sqrt()
}

fn sq_dist(a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)
}

/// Where a contour point sits on the mesh — the chaining identity. Two
/// segments join exactly when they end on the same mesh edge (or vertex), so
/// keying by topology makes the chaining exact and deterministic (no
/// coordinate epsilon matching).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Anchor {
    /// The contour passes exactly through a mesh vertex.
    Vertex(u32),
    /// The contour crosses the interior of the undirected edge `(lo, hi)`.
    Edge(u32, u32),
}

/// Extract iso-lines from a triangle mesh with per-node values.
///
/// Per level, each triangle whose three values are finite is contoured by
/// linear interpolation along its edges (a triangle touching a `NaN` node is
/// skipped — holes break lines, they never bend them). The resulting segments
/// are chained into polylines by their mesh-edge anchors; output order and the
/// chaining itself are deterministic (ordered maps, ordered walk).
///
/// Returns one `(level, polylines)` pair per requested level, in the given
/// order; each polyline is a list of `[x, y]` vertices (closed loops repeat
/// their first point last).
pub fn contour_trimesh(
    nodes: &[[f64; 2]],
    triangles: &[[u32; 3]],
    values: &[f64],
    levels: &[f64],
) -> Vec<(f64, Vec<Vec<[f64; 2]>>)> {
    levels
        .iter()
        .map(|&level| (level, contour_one(nodes, triangles, values, level)))
        .collect()
}

fn contour_one(
    nodes: &[[f64; 2]],
    triangles: &[[u32; 3]],
    values: &[f64],
    level: f64,
) -> Vec<Vec<[f64; 2]>> {
    let mut points: BTreeMap<Anchor, [f64; 2]> = BTreeMap::new();
    let mut segments: BTreeSet<(Anchor, Anchor)> = BTreeSet::new();

    for t in triangles {
        let v = [
            values[t[0] as usize],
            values[t[1] as usize],
            values[t[2] as usize],
        ];
        // NaN-aware: a triangle touching an undefined node is skipped whole.
        if v.iter().any(|x| x.is_nan()) {
            continue;
        }
        let above = [v[0] >= level, v[1] >= level, v[2] >= level];
        if above.iter().all(|&a| a) || above.iter().all(|&a| !a) {
            continue;
        }
        // Exactly two of the three edges cross the level.
        let mut ends: Vec<Anchor> = Vec::with_capacity(2);
        for (ka, kb) in [(0usize, 1usize), (1, 2), (2, 0)] {
            if above[ka] == above[kb] {
                continue;
            }
            let (a, b) = (t[ka], t[kb]);
            let (va, vb) = (v[ka], v[kb]);
            let frac = (level - va) / (vb - va);
            let anchor = if frac <= 0.0 {
                Anchor::Vertex(a)
            } else if frac >= 1.0 {
                Anchor::Vertex(b)
            } else {
                Anchor::Edge(a.min(b), a.max(b))
            };
            let p = match anchor {
                Anchor::Vertex(n) => nodes[n as usize],
                Anchor::Edge(..) => {
                    let (pa, pb) = (nodes[a as usize], nodes[b as usize]);
                    [
                        pa[0] + frac * (pb[0] - pa[0]),
                        pa[1] + frac * (pb[1] - pa[1]),
                    ]
                }
            };
            points.entry(anchor).or_insert(p);
            ends.push(anchor);
        }
        if let [e1, e2] = ends[..] {
            if e1 != e2 {
                segments.insert(if e1 <= e2 { (e1, e2) } else { (e2, e1) });
            }
        }
    }

    chain(&points, segments)
}

/// Chain segments into polylines: open chains first (started at odd-degree
/// anchors in ascending order), then closed loops (started at their smallest
/// anchor). At a junction the walk always takes the smallest unused neighbour,
/// so the output is fully deterministic.
fn chain(
    points: &BTreeMap<Anchor, [f64; 2]>,
    segments: BTreeSet<(Anchor, Anchor)>,
) -> Vec<Vec<[f64; 2]>> {
    let mut adjacency: BTreeMap<Anchor, Vec<Anchor>> = BTreeMap::new();
    for &(a, b) in &segments {
        adjacency.entry(a).or_default().push(b);
        adjacency.entry(b).or_default().push(a);
    }
    for nbrs in adjacency.values_mut() {
        nbrs.sort_unstable();
    }

    let mut unused = segments;
    let mut out: Vec<Vec<[f64; 2]>> = Vec::new();

    let walk_from = |start: Anchor, unused: &mut BTreeSet<(Anchor, Anchor)>| {
        let mut line = vec![points[&start]];
        let mut current = start;
        loop {
            let next = adjacency[&current].iter().copied().find(|&n| {
                let key = if current <= n {
                    (current, n)
                } else {
                    (n, current)
                };
                unused.contains(&key)
            });
            match next {
                Some(n) => {
                    let key = if current <= n {
                        (current, n)
                    } else {
                        (n, current)
                    };
                    unused.remove(&key);
                    line.push(points[&n]);
                    current = n;
                }
                None => break,
            }
        }
        line
    };

    // Open chains: walk from every odd-degree anchor.
    let odd: Vec<Anchor> = adjacency
        .iter()
        .filter(|(_, n)| n.len() % 2 == 1)
        .map(|(a, _)| *a)
        .collect();
    for start in odd {
        while adjacency[&start].iter().any(|&n| {
            let key = if start <= n { (start, n) } else { (n, start) };
            unused.contains(&key)
        }) {
            out.push(walk_from(start, &mut unused));
        }
    }
    // Closed loops: whatever remains.
    while let Some(&(a, _)) = unused.iter().next() {
        out.push(walk_from(a, &mut unused));
    }

    out.retain(|line| line.len() >= 2);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    #[test]
    fn dip_kernel_recovers_world_gradient_from_rotated_flipped_lattice() {
        let (gx, gy) = (0.2_f64, -0.1_f64);
        let rotation_deg = 37.0_f64;
        let (sin_theta, cos_theta) = rotation_deg.to_radians().sin_cos();
        let du = gx * cos_theta + gy * sin_theta;
        let dv = -gx * sin_theta + gy * cos_theta;
        let (x_step, y_step) = (2.0, -3.0); // y-flipped J axis
        let mut values = Array2::zeros((4, 5));
        for j in 0..5 {
            for i in 0..4 {
                values[[i, j]] = du * i as f64 * x_step + dv * j as f64 * y_step;
            }
        }

        let (angle, azimuth) = dip_fields(&values, x_step, y_step, rotation_deg);
        let expected_angle = gx.hypot(gy).atan().to_degrees();
        let expected_azimuth = (-gx).atan2(-gy).to_degrees().rem_euclid(360.0);
        for &v in &angle {
            assert_relative_eq!(v, expected_angle, epsilon = 1e-12);
        }
        for &v in &azimuth {
            assert_relative_eq!(v, expected_azimuth, epsilon = 1e-12);
        }
    }

    /// A unit-square split into two triangles along (0,0)–(1,1), values = x.
    fn square() -> (Vec<[f64; 2]>, Vec<[u32; 3]>, Vec<f64>) {
        let nodes = vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]];
        let triangles = vec![[0, 1, 3], [0, 3, 2]];
        let values = vec![0.0, 1.0, 0.0, 1.0];
        (nodes, triangles, values)
    }

    #[test]
    fn aligned_levels_snap_to_multiples() {
        assert_eq!(
            aligned_levels(3.2, 17.8, 5.0).unwrap(),
            vec![5.0, 10.0, 15.0]
        );
        assert_eq!(
            aligned_levels(-7.0, 7.0, 5.0).unwrap(),
            vec![-5.0, 0.0, 5.0]
        );
        // Endpoints that are exact multiples stay included.
        assert_eq!(
            aligned_levels(5.0, 15.0, 5.0).unwrap(),
            vec![5.0, 10.0, 15.0]
        );
        assert!(aligned_levels(0.0, 1.0, 0.0).is_err());
        assert!(aligned_levels(0.0, 1.0, -2.0).is_err());
        assert!(aligned_levels(f64::NAN, 1.0, 5.0).unwrap().is_empty());
    }

    #[test]
    fn contours_a_planar_field_with_one_straight_line() {
        let (nodes, triangles, values) = square();
        let out = contour_trimesh(&nodes, &triangles, &values, &[0.25]);
        assert_eq!(out.len(), 1);
        let (level, lines) = &out[0];
        assert_eq!(*level, 0.25);
        // One chained polyline crossing both triangles.
        assert_eq!(
            lines.len(),
            1,
            "segments must chain into one line: {lines:?}"
        );
        for p in &lines[0] {
            assert_relative_eq!(p[0], 0.25, epsilon = 1e-12);
        }
        // Spans the full y range.
        let ys: Vec<f64> = lines[0].iter().map(|p| p[1]).collect();
        assert_relative_eq!(ys.iter().cloned().fold(f64::INFINITY, f64::min), 0.0);
        assert_relative_eq!(ys.iter().cloned().fold(f64::NEG_INFINITY, f64::max), 1.0);
    }

    #[test]
    fn nan_breaks_lines_instead_of_bending_them() {
        // A 1 x 2 vertical strip of cells, values = x. The 0.25-contour crosses
        // both cells as one straight line; a NaN node in the upper cell must
        // *shorten* it (skip those triangles), never bend it.
        let nodes = vec![
            [0.0, 0.0],
            [1.0, 0.0],
            [0.0, 1.0],
            [1.0, 1.0],
            [0.0, 2.0],
            [1.0, 2.0],
        ];
        let triangles = vec![[0, 1, 3], [0, 3, 2], [2, 3, 5], [2, 5, 4]];
        let values = vec![0.0, 1.0, 0.0, 1.0, 0.0, 1.0];

        let full = contour_trimesh(&nodes, &triangles, &values, &[0.25]);
        assert_eq!(full[0].1.len(), 1);
        let ys: Vec<f64> = full[0].1[0].iter().map(|p| p[1]).collect();
        assert_relative_eq!(ys.iter().cloned().fold(f64::NEG_INFINITY, f64::max), 2.0);

        let mut holed = values.clone();
        holed[5] = f64::NAN; // kills both upper-cell triangles
        let out = contour_trimesh(&nodes, &triangles, &holed, &[0.25]);
        let lines = &out[0].1;
        assert_eq!(lines.len(), 1, "the line is cut short, not removed");
        for p in &lines[0] {
            assert_relative_eq!(p[0], 0.25, epsilon = 1e-12); // straight, not bent
            assert!(p[1] <= 1.0 + 1e-12, "no point may enter the NaN cell");
        }
    }

    #[test]
    fn a_level_through_a_vertex_is_handled_once() {
        // Level exactly at a node value: the anchor collapses to the vertex and
        // adjacent triangles agree on it — no duplicate or zero-length segment.
        let nodes = vec![[0.0, 0.0], [1.0, 0.0], [2.0, 0.0], [1.0, 1.0]];
        let triangles = vec![[0, 1, 3], [1, 2, 3]];
        let values = vec![0.0, 1.0, 2.0, 0.0];
        let out = contour_trimesh(&nodes, &triangles, &values, &[1.0]);
        let lines = &out[0].1;
        assert_eq!(lines.len(), 1);
        // The line passes exactly through node 1.
        assert!(lines[0]
            .iter()
            .any(|p| (p[0] - 1.0).abs() < 1e-12 && p[1].abs() < 1e-12));
    }

    #[test]
    fn no_lines_outside_the_value_range() {
        let (nodes, triangles, values) = square();
        let out = contour_trimesh(&nodes, &triangles, &values, &[5.0, -3.0]);
        assert!(out.iter().all(|(_, lines)| lines.is_empty()));
    }

    #[test]
    fn dp_collapses_a_noisy_straight_line_to_its_endpoints() {
        // 21 points on y = 0 with |noise| < 0.05; tol = 0.1 collapses to the two ends.
        let noise = [
            0.0, 0.03, -0.02, 0.04, -0.01, 0.02, -0.03, 0.01, -0.04, 0.02, 0.0,
        ];
        let pts: Vec<[f64; 2]> = (0..11).map(|k| [k as f64, noise[k]]).collect();
        let out = douglas_peucker(&pts, 0.1);
        assert_eq!(
            out.len(),
            2,
            "a within-tol straight line keeps only its ends"
        );
        assert_eq!(out[0], pts[0]);
        assert_eq!(out[1], pts[10]);
    }

    #[test]
    fn dp_preserves_an_l_shape_corner() {
        // A right-angle: the corner is far from the chord and must survive.
        let pts = vec![
            [0.0, 0.0],
            [1.0, 0.0],
            [2.0, 0.0],
            [3.0, 0.0], // corner
            [3.0, 1.0],
            [3.0, 2.0],
            [3.0, 3.0],
        ];
        let out = douglas_peucker(&pts, 0.1);
        assert_eq!(out, vec![[0.0, 0.0], [3.0, 0.0], [3.0, 3.0]]);
    }

    #[test]
    fn dp_keeps_open_endpoints_and_ring_closure() {
        // Open line of two points is a no-op.
        let two = vec![[0.0, 0.0], [5.0, 0.0]];
        assert_eq!(douglas_peucker(&two, 1.0), two);

        // A square ring (5 pts, closed) with a within-tol midpoint per side stays
        // a ring (>= 4 points) and keeps its four corners + closure.
        let ring = vec![
            [0.0, 0.0],
            [5.0, 0.01],
            [10.0, 0.0],
            [10.0, 10.0],
            [0.0, 10.0],
            [0.0, 0.0],
        ];
        let out = douglas_peucker(&ring, 0.1);
        assert!(out.len() >= 4, "a ring never drops below four points");
        assert_eq!(out.first(), out.last(), "closure is preserved");
        // The collinear midpoint [5, 0.01] is dropped; the four corners remain.
        assert!(out.iter().all(|p| *p != [5.0, 0.01]));

        // tol <= 0 is a no-op.
        assert_eq!(douglas_peucker(&ring, 0.0), ring);
    }
}
