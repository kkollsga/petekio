//! `algorithms::surfaces` — pure surface-numeric kernels: iso-line
//! (contour) extraction over a triangle mesh with per-node values.
//!
//! Type-light by the constitution: primitives in (`nodes`/`triangles`/`values`
//! slices), primitives out. Domain types (`Surface`, `StructuredMeshSurface`,
//! `TriSurface`) call in through their shells; the marching-triangles math has
//! this one home.

use crate::foundation::{GeoError, Result};
use std::collections::{BTreeMap, BTreeSet};

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
}
