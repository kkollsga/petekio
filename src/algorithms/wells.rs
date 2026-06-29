//! `algorithms::wells` — well-path numerics. The **minimum-curvature** method:
//! turn a directional survey (MD / inclination / azimuth) into positions, and
//! interpolate position anywhere along the arc between two stations.
//!
//! Pure, type-light kernel (primitives + [`Point3`] in/out — no `Well`/`Station`
//! or IO coupling) so it is trivial to QC in isolation and cheap to lift into the
//! external **petekAlgorithms** library. One formula, one home: a full
//! station-to-station step is just [`arc_point`] at `f = 1`, so [`survey_positions`]
//! and mid-station interpolation share the exact same math.

use crate::foundation::{GeoError, Point3, Result};

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
}
