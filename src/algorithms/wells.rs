//! `algorithms::wells` — well-path numerics. Two kernel families:
//!
//! - **Minimum-curvature** trajectory: turn a directional survey (MD /
//!   inclination / azimuth) into positions, and interpolate position anywhere
//!   along the arc between two stations.
//! - **Stratigraphic-order merge** ([`merge_strat_order`]): fuse the per-well
//!   top sequences from a multi-well tops file into one global
//!   lithostratigraphic column.
//!
//! Pure, type-light kernels (primitives + [`Point3`] in/out — no `Well`/`Station`
//! or IO coupling) so they are trivial to QC in isolation and cheap to lift into
//! the external **petekTools** library. One formula, one home: a full
//! station-to-station step is just [`arc_point`] at `f = 1`, so [`survey_positions`]
//! and mid-station interpolation share the exact same math.

use crate::foundation::{GeoError, Point3, Result};
use std::collections::{BTreeSet, HashMap};

/// Below this dogleg (radians) the ratio factor is taken from its Taylor
/// expansion `RF ≈ 1 + β²/12` to avoid the `0/0` in `(2/β)·tan(β/2)`.
const SMALL_BETA: f64 = 1e-4;

/// Unit tangent `[north, east, down]` for an inclination/azimuth (degrees,
/// inclination from vertical, azimuth clockwise from North).
pub fn tangent(inc_deg: f64, azi_deg: f64) -> [f64; 3] {
    let (i, a) = (inc_deg.to_radians(), azi_deg.to_radians());
    [i.sin() * a.cos(), i.sin() * a.sin(), i.cos()]
}

/// Dogleg angle (radians) between two unit tangents.
pub fn dogleg(t1: [f64; 3], t2: [f64; 3]) -> f64 {
    (t1[0] * t2[0] + t1[1] * t2[1] + t1[2] * t2[2])
        .clamp(-1.0, 1.0)
        .acos()
}

/// Minimum-curvature ratio factor for a dogleg `beta` (radians):
/// `(2/β)·tan(β/2)`, → 1 as β → 0.
pub fn ratio_factor(beta: f64) -> f64 {
    if beta < SMALL_BETA {
        1.0 + beta * beta / 12.0
    } else {
        (2.0 / beta) * (beta / 2.0).tan()
    }
}

/// Position at fraction `f ∈ [0, 1]` of the minimum-curvature arc that starts at
/// `pa` with unit tangent `t1` and ends at the next station's tangent `t2`, over
/// MD span `dmd`. The tangent at `f` is the slerp of `t1`/`t2`; the partial
/// dogleg is `f·β`. `f = 0` → `pa`; `f = 1` → the next station's position.
pub fn arc_point(pa: Point3, t1: [f64; 3], t2: [f64; 3], f: f64, dmd: f64) -> Point3 {
    let beta = dogleg(t1, t2);
    let tf = if beta < SMALL_BETA {
        let l = [
            t1[0] + (t2[0] - t1[0]) * f,
            t1[1] + (t2[1] - t1[1]) * f,
            t1[2] + (t2[2] - t1[2]) * f,
        ];
        let n = (l[0] * l[0] + l[1] * l[1] + l[2] * l[2]).sqrt().max(1e-300);
        [l[0] / n, l[1] / n, l[2] / n]
    } else {
        let s = beta.sin();
        let (w1, w2) = (((1.0 - f) * beta).sin() / s, (f * beta).sin() / s);
        [
            w1 * t1[0] + w2 * t2[0],
            w1 * t1[1] + w2 * t2[1],
            w1 * t1[2] + w2 * t2[2],
        ]
    };
    let half = 0.5 * (f * dmd) * ratio_factor(f * beta);
    // p.y ← north, p.x ← east, p.z ← down.
    Point3::new(
        pa.x + half * (t1[1] + tf[1]),
        pa.y + half * (t1[0] + tf[0]),
        pa.z + half * (t1[2] + tf[2]),
    )
}

/// Minimum-curvature positions for a directional survey. `stations` is
/// `(md, inc_deg, azi_deg)` rows with strictly increasing MD; returns one
/// `(position, unit_tangent)` per station, accumulating from the wellhead
/// `head = (x, y)` and KB datum `kb` (KB at `z = -kb`; the first station sits at
/// `z = md₀ - kb`, so a vertical hole gives `z = md - kb`). A station step is
/// [`arc_point`] at `f = 1`.
pub fn survey_positions(
    stations: &[(f64, f64, f64)],
    head: (f64, f64),
    kb: f64,
) -> Result<Vec<(Point3, [f64; 3])>> {
    let &(md0, inc0, azi0) = stations
        .first()
        .ok_or_else(|| GeoError::OutOfRange("trajectory needs at least one station".into()))?;
    let mut pos = Point3::new(head.0, head.1, md0 - kb);
    let mut t_prev = tangent(inc0, azi0);
    let mut out = Vec::with_capacity(stations.len());
    out.push((pos, t_prev));
    for w in stations.windows(2) {
        let (md_a, _, _) = w[0];
        let (md_b, inc_b, azi_b) = w[1];
        let dmd = md_b - md_a;
        if dmd <= 0.0 {
            return Err(GeoError::OutOfRange(
                "station measured depth must strictly increase".into(),
            ));
        }
        let t_b = tangent(inc_b, azi_b);
        pos = arc_point(pos, t_prev, t_b, 1.0, dmd);
        out.push((pos, t_b));
        t_prev = t_b;
    }
    Ok(out)
}

/// Per-sample depth weights for thickness-weighting a curve: each sample carries
/// the MD span it represents under the **midpoint rule** — half the gap to each
/// neighbour, with the full end step at the two extremes. Uniform sampling →
/// every weight equal → identical to a plain (unweighted) mean, so weighting only
/// changes irregular / mixed-rate logs. `0` samples → empty; `1` sample → `[1.0]`.
///
/// This is the one home for the zone-table thickness weighting; the `Surface`/
/// well bindings call in rather than inlining the stencil. (It differs from
/// [`crate::analysis::interpret`]'s net-pay representative thickness, which halves
/// the end steps to make the weights sum to the total span — a different
/// convention for a different purpose.)
pub fn dz_weights(md: &[f64]) -> Vec<f64> {
    let n = md.len();
    match n {
        0 => Vec::new(),
        1 => vec![1.0],
        _ => (0..n)
            .map(|i| {
                if i == 0 {
                    md[1] - md[0]
                } else if i == n - 1 {
                    md[n - 1] - md[n - 2]
                } else {
                    (md[i + 1] - md[i - 1]) / 2.0
                }
            })
            .collect(),
    }
}

/// Merge per-well top sequences into one global lithostratigraphic order.
///
/// Each entry of `wells` is one well's picks as `(md, name)` in **file order**.
/// A name `A` precedes `B` whenever `A` is *strictly shallower* than `B` in
/// **some** well; equal MD — a zero-thickness pinch-out — yields no constraint.
/// These strict constraints are merged by a topological sort, so a well that
/// develops a marker resolves an order that a well where it pinches out cannot.
/// Ties the data leaves unresolved (picks coincident in *every* well) break by
/// **first appearance** across `wells` (formation tops typically precede their
/// appended members), then by insertion. Contradictory constraints (a cycle)
/// neither hang nor panic: once no zero-in-degree node remains, the rest are
/// emitted in first-appearance order.
///
/// The caller (manager) gathers *every* well's Horizon picks from the tops file
/// — including wells not loaded into the project — so the returned column
/// reflects the whole field, not one borehole. Names are returned once each, in
/// global lithostratigraphic order.
///
/// `hints` are user-supplied soft precedence edges `(above, below)` — "place
/// `above` shallower than `below`". A hint is applied **only where the data
/// leaves the pair unordered**: if some well's MD already orders them the *other*
/// way, the hint is dropped (real positions always win). Hints thus break
/// stalemates — pairs coincident in every well — without ever overriding
/// geology. Names not present in any well are ignored.
pub fn merge_strat_order(wells: &[Vec<(f64, &str)>], hints: &[(&str, &str)]) -> Vec<String> {
    // First-appearance index doubles as the tiebreak key (file order, as passed).
    let mut appearance: Vec<&str> = Vec::new();
    let mut index: HashMap<&str, usize> = HashMap::new();
    for well in wells {
        for &(_, name) in well {
            index.entry(name).or_insert_with(|| {
                appearance.push(name);
                appearance.len() - 1
            });
        }
    }
    let n = appearance.len();
    if n == 0 {
        return Vec::new();
    }

    // Strict precedence edges: A≺B if A strictly shallower than B in some well.
    let mut edges: BTreeSet<(usize, usize)> = BTreeSet::new();
    for well in wells {
        for (i, &(md_a, a)) in well.iter().enumerate() {
            for &(md_b, b) in &well[i + 1..] {
                if a == b {
                    continue;
                }
                let (ia, ib) = (index[a], index[b]);
                if md_a < md_b {
                    edges.insert((ia, ib));
                } else if md_b < md_a {
                    edges.insert((ib, ia));
                } // equal MD → no constraint
            }
        }
    }

    // Apply hints, but only where the *data* doesn't already order the pair the
    // other way. Reachability is computed over data edges only, so a hint can
    // never reverse a strict MD relationship — it just fills a stalemate.
    if !hints.is_empty() {
        let mut data_succ: Vec<Vec<usize>> = vec![Vec::new(); n];
        for &(from, to) in &edges {
            data_succ[from].push(to);
        }
        let reaches = |from: usize, to: usize| -> bool {
            let mut seen = vec![false; n];
            let mut stack = vec![from];
            seen[from] = true;
            while let Some(u) = stack.pop() {
                for &v in &data_succ[u] {
                    if v == to {
                        return true;
                    }
                    if !seen[v] {
                        seen[v] = true;
                        stack.push(v);
                    }
                }
            }
            false
        };
        for &(above, below) in hints {
            let (Some(&ia), Some(&ib)) = (index.get(above), index.get(below)) else {
                continue; // a name not present in any well — nothing to order
            };
            // Drop the hint if data already puts `below` shallower than `above`.
            if ia != ib && !reaches(ib, ia) {
                edges.insert((ia, ib));
            }
        }
    }

    let mut indeg = vec![0usize; n];
    let mut succ: Vec<Vec<usize>> = vec![Vec::new(); n];
    for &(from, to) in &edges {
        succ[from].push(to);
        indeg[to] += 1;
    }

    // Kahn topo-sort. Node id == first-appearance index, so scanning `0..n` for
    // the first eligible node is exactly the first-appearance tiebreak. When no
    // zero-in-degree node remains (a cycle), fall back to the lowest unplaced
    // node — deterministic, never blocks.
    let mut placed = vec![false; n];
    let mut order: Vec<usize> = Vec::with_capacity(n);
    while order.len() < n {
        let k = (0..n)
            .find(|&k| !placed[k] && indeg[k] == 0)
            .or_else(|| (0..n).find(|&k| !placed[k]))
            .expect("an unplaced node exists while order.len() < n");
        placed[k] = true;
        order.push(k);
        for &m in &succ[k] {
            if !placed[m] && indeg[m] > 0 {
                indeg[m] -= 1;
            }
        }
    }
    order
        .into_iter()
        .map(|k| appearance[k].to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;
    use std::f64::consts::{FRAC_PI_2, PI};

    #[test]
    fn ratio_factor_limits() {
        assert_relative_eq!(ratio_factor(0.0), 1.0, epsilon = 1e-12);
        assert_relative_eq!(ratio_factor(1e-6), 1.0, epsilon = 1e-10);
        // (2/(π/2))·tan(π/4) = (4/π)·1.
        assert_relative_eq!(ratio_factor(FRAC_PI_2), 4.0 / PI, epsilon = 1e-12);
    }

    #[test]
    fn tangent_and_dogleg() {
        assert_relative_eq!(tangent(0.0, 0.0)[2], 1.0); // vertical → straight down
        let east = tangent(90.0, 90.0);
        assert_relative_eq!(east[1], 1.0, epsilon = 1e-12); // due-east horizontal
                                                            // vertical vs horizontal = 90° dogleg.
        assert_relative_eq!(
            dogleg(tangent(0.0, 0.0), tangent(90.0, 0.0)),
            FRAC_PI_2,
            epsilon = 1e-12
        );
    }

    #[test]
    fn arc_point_endpoints_and_vertical() {
        let pa = Point3::new(10.0, 20.0, 100.0);
        let down = [0.0, 0.0, 1.0];
        // f = 0 returns the start.
        let p0 = arc_point(pa, down, down, 0.0, 50.0);
        assert_relative_eq!(p0.z, 100.0, epsilon = 1e-12);
        // vertical 50 m → straight down.
        let p1 = arc_point(pa, down, down, 1.0, 50.0);
        assert_relative_eq!(p1.z, 150.0, epsilon = 1e-12);
        assert_relative_eq!(p1.x, 10.0, epsilon = 1e-12);
    }

    #[test]
    fn survey_vertical_is_md_minus_kb() {
        let pos =
            survey_positions(&[(0.0, 0.0, 0.0), (1000.0, 0.0, 0.0)], (5.0, 6.0), 30.0).unwrap();
        assert_relative_eq!(pos[0].0.z, -30.0, epsilon = 1e-12);
        assert_relative_eq!(pos[1].0.z, 970.0, epsilon = 1e-12); // 1000 - 30
        assert_relative_eq!(pos[1].0.x, 5.0, epsilon = 1e-12);
    }

    #[test]
    fn survey_rejects_non_increasing_md() {
        assert!(
            survey_positions(&[(100.0, 0.0, 0.0), (100.0, 5.0, 0.0)], (0.0, 0.0), 0.0).is_err()
        );
        assert!(survey_positions(&[], (0.0, 0.0), 0.0).is_err());
    }

    #[test]
    fn dz_weights_edges_and_uniform() {
        assert!(dz_weights(&[]).is_empty());
        assert_eq!(dz_weights(&[5.0]), vec![1.0]);
        // Uniform spacing → all weights equal to the step.
        assert_eq!(
            dz_weights(&[0.0, 10.0, 20.0, 30.0]),
            vec![10.0, 10.0, 10.0, 10.0]
        );
        // Irregular: ends carry the full end step, interior the half-span.
        // md = [0, 10, 40] → [10, (40-0)/2=20, 30]
        assert_eq!(dz_weights(&[0.0, 10.0, 40.0]), vec![10.0, 20.0, 30.0]);
    }

    // ---- merge_strat_order -------------------------------------------------

    #[test]
    fn strat_empty_is_empty() {
        assert!(merge_strat_order(&[], &[]).is_empty());
        assert!(merge_strat_order(&[vec![]], &[]).is_empty());
    }

    #[test]
    fn strat_single_well_recovers_md_order_from_any_file_order() {
        // File order scrambled; MD order is A<B<C. Edges come from MD, not file.
        let w = vec![(30.0, "C"), (10.0, "A"), (20.0, "B")];
        assert_eq!(merge_strat_order(&[w], &[]), ["A", "B", "C"]);
    }

    #[test]
    fn strat_separation_resolves_a_coincident_tie() {
        // Well 1 leaves B,C coincident (zero thickness); well 2 develops B<C.
        // The merge must take the order well 2 supplies.
        let w1 = vec![(10.0, "A"), (20.0, "B"), (20.0, "C")];
        let w2 = vec![(10.0, "A"), (20.0, "B"), (30.0, "C")];
        assert_eq!(merge_strat_order(&[w1, w2], &[]), ["A", "B", "C"]);
    }

    #[test]
    fn strat_tie_everywhere_breaks_by_first_appearance() {
        // X,Y coincident in every well → no edge. First appearance decides.
        let xy = vec![vec![(10.0, "X"), (10.0, "Y")], vec![(5.0, "X"), (5.0, "Y")]];
        assert_eq!(merge_strat_order(&xy, &[]), ["X", "Y"]);
        // Reverse first appearance → reversed result (the tiebreak, not MD).
        let yx = vec![vec![(10.0, "Y"), (10.0, "X")], vec![(5.0, "Y"), (5.0, "X")]];
        assert_eq!(merge_strat_order(&yx, &[]), ["Y", "X"]);
    }

    #[test]
    fn strat_contradiction_is_deterministic_and_never_hangs() {
        // Well 1: P<Q; well 2: Q<P. A cycle — must return both, deterministically,
        // in first-appearance order (P seen first), without panicking.
        let w1 = vec![(10.0, "P"), (20.0, "Q")];
        let w2 = vec![(10.0, "Q"), (20.0, "P")];
        assert_eq!(merge_strat_order(&[w1, w2], &[]), ["P", "Q"]);
    }

    #[test]
    fn strat_field_shape_coincident_pairs() {
        // Mirrors the real shape (synthetic names): one well leaves the Mid/Lower
        // pair coincident, another develops Mid<Lower; a sand listed last in the
        // file sits at its resolved depth, not at the end.
        let absent = vec![(100.0, "Top"), (120.0, "Mid"), (120.0, "Lower")];
        let dev = vec![(100.0, "Top"), (120.0, "Mid"), (130.0, "Lower")];
        let sand = vec![(100.0, "Top"), (110.0, "Sand"), (120.0, "Mid")];
        // "Sand" appears last across the inputs but its MD places it above Mid.
        let order = merge_strat_order(&[absent, dev, sand], &[]);
        assert_eq!(order, ["Top", "Sand", "Mid", "Lower"]);
    }

    #[test]
    fn hint_breaks_a_stalemate() {
        // X,Y coincident in every well → no data edge. First appearance puts X
        // first; a hint "Y above X" overrides only this stalemate.
        let wells = vec![vec![(10.0, "X"), (10.0, "Y")], vec![(5.0, "X"), (5.0, "Y")]];
        assert_eq!(merge_strat_order(&wells, &[]), ["X", "Y"]); // tiebreak default
        assert_eq!(merge_strat_order(&wells, &[("Y", "X")]), ["Y", "X"]); // hint wins
    }

    #[test]
    fn hint_contradicting_data_is_ignored() {
        // Data: A strictly above B (developed in the well). A hint trying to put
        // B above A must be dropped — real positions win.
        let well = vec![(100.0, "A"), (120.0, "B")];
        let one = std::slice::from_ref(&well);
        assert_eq!(merge_strat_order(one, &[]), ["A", "B"]);
        assert_eq!(merge_strat_order(one, &[("B", "A")]), ["A", "B"]); // ignored
    }

    #[test]
    fn hint_propagates_transitively() {
        // Z is coincident with Y in every well (stalemate), and data gives X≺Y.
        // A hint "W above X" with W coincident-everywhere drags W (and nothing
        // below it) above the whole X/Y/Z block.
        let w1 = vec![(10.0, "X"), (20.0, "Y"), (20.0, "Z")];
        let w2 = vec![(10.0, "X"), (10.0, "W"), (20.0, "Y")];
        // Without a hint W floats by first appearance (after X). Hint: W above X.
        let order = merge_strat_order(&[w1, w2], &[("W", "X")]);
        assert_eq!(order.first().map(String::as_str), Some("W"));
        assert!(
            order.iter().position(|s| s == "W").unwrap()
                < order.iter().position(|s| s == "X").unwrap()
        );
    }
}
