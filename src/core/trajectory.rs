//! `Trajectory` — a well path normalized to a positioned `md → (x, y, z)` curve.
//!
//! Every survey-input variant ([`TrajectoryInput`]) is reduced to a single
//! positioned path through the **minimum-curvature** method (`Hold`/`Steer` are
//! integrated to stations first; `Xyz` is taken as explicit positions). The
//! resulting nodes carry measured depth `md` and a [`Point3`] whose `z` is the
//! subsea true vertical depth (TVDSS, positive **downward**).
//!
//! Depth convention: positions accumulate from the wellhead `head = (x, y)` and
//! the kelly-bushing elevation `kb`. The KB sits at `z = -kb` (above the mean
//! sea-level datum), measured depth runs from the KB, and the path above the
//! first station is assumed vertical, so the first station sits at
//! `z = md₀ - kb`. A vertical well therefore satisfies `tvd(md) = md - kb`.

use crate::foundation::{GeoError, Point3, Result};

/// A directional-survey station: measured depth with inclination and azimuth.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Station {
    /// Measured depth along the hole.
    pub md: f64,
    /// Inclination from vertical, in degrees (0 = vertical).
    pub inc_deg: f64,
    /// Azimuth clockwise from North, in degrees.
    pub azi_deg: f64,
}

impl Station {
    /// A station at `md` with the given inclination and azimuth (degrees).
    pub fn new(md: f64, inc_deg: f64, azi_deg: f64) -> Self {
        Self {
            md,
            inc_deg,
            azi_deg,
        }
    }
}

/// The survey-input variants, each normalized to a positioned path.
#[derive(Debug, Clone)]
pub enum TrajectoryInput {
    /// Explicit positions, used directly (`md` = cumulative chord length).
    Xyz(Vec<Point3>),
    /// MD/inclination/azimuth stations → minimum-curvature.
    MdIncAzi(Vec<Station>),
    /// MD/inclination/azimuth stations → minimum-curvature (alias).
    Stations(Vec<Station>),
    /// A constant inclination/azimuth segment from `from` to `to_md`.
    Hold { from: Station, to_md: f64 },
    /// A build/turn-rate segment (degrees per 100 MD) from `from` to `to_md`.
    Steer {
        from: Station,
        build_per_100: f64,
        turn_per_100: f64,
        to_md: f64,
    },
}

/// One positioned node on the path. `dir` is the unit tangent
/// `[north, east, down]` from the station's inc/azi, present for survey-derived
/// nodes (it drives arc-consistent interpolation between stations) and `None`
/// for explicit `Xyz` paths (which fall back to straight-line interpolation).
#[derive(Debug, Clone, Copy)]
struct Node {
    md: f64,
    p: Point3,
    dir: Option<[f64; 3]>,
}

/// A normalized, positioned well path: a monotone `md → (x, y, z)` curve with
/// interpolation. `z` is subsea TVD, positive downward.
#[derive(Debug, Clone)]
pub struct Trajectory {
    nodes: Vec<Node>,
}

/// Below this dogleg (radians) the ratio factor is taken from its Taylor
/// expansion `RF ≈ 1 + β²/12` to avoid the `0/0` in `(2/β)·tan(β/2)`.
const SMALL_BETA: f64 = 1e-4;

/// MD step (project units) used to integrate a `Steer` segment into stations.
const STEER_STEP: f64 = 30.0;

impl Trajectory {
    /// Normalize a survey input into a positioned path, accumulating from the
    /// wellhead `head` and datum `kb`. `Err` on empty or non-increasing input.
    pub fn from_input(input: TrajectoryInput, head: (f64, f64), kb: f64) -> Result<Self> {
        let nodes = match input {
            TrajectoryInput::Xyz(pts) => nodes_from_xyz(pts)?,
            TrajectoryInput::MdIncAzi(s) | TrajectoryInput::Stations(s) => {
                min_curvature(&s, head, kb)?
            }
            TrajectoryInput::Hold { from, to_md } => {
                let end = Station::new(to_md, from.inc_deg, from.azi_deg);
                min_curvature(&[from, end], head, kb)?
            }
            TrajectoryInput::Steer {
                from,
                build_per_100,
                turn_per_100,
                to_md,
            } => {
                let stations = steer_stations(from, build_per_100, turn_per_100, to_md);
                min_curvature(&stations, head, kb)?
            }
        };
        Ok(Trajectory { nodes })
    }

    /// The `(min, max)` measured-depth span of the path.
    pub fn md_range(&self) -> (f64, f64) {
        match (self.nodes.first(), self.nodes.last()) {
            (Some(a), Some(b)) => (a.md, b.md),
            _ => (f64::NAN, f64::NAN),
        }
    }

    /// Interpolated position at measured depth `md`, or `None` outside the
    /// path's `md_range`. Between two survey stations the position is taken along
    /// the **minimum-curvature arc** (slerp of the station tangents); an explicit
    /// `Xyz` path (no tangents) falls back to straight-line interpolation.
    pub fn xyz(&self, md: f64) -> Option<Point3> {
        let (lo, hi) = self.md_range();
        if md.is_nan() || md < lo || md > hi {
            return None;
        }
        for w in self.nodes.windows(2) {
            let (a, b) = (w[0], w[1]);
            if md >= a.md && md <= b.md {
                let span = b.md - a.md;
                if span <= 0.0 {
                    return Some(a.p);
                }
                let f = (md - a.md) / span;
                return Some(match (a.dir, b.dir) {
                    (Some(t1), Some(t2)) => arc_point(a.p, t1, t2, f, span),
                    _ => lerp3(a.p, b.p, f),
                });
            }
        }
        // Single-node path: only the exact MD resolves.
        self.nodes.first().filter(|n| n.md == md).map(|n| n.p)
    }

    /// Subsea true vertical depth at measured depth `md`, or `None` outside
    /// `md_range`.
    pub fn tvd(&self, md: f64) -> Option<f64> {
        self.xyz(md).map(|p| p.z)
    }

    /// Measured depth at a given TVD — the **shallowest** (smallest-MD)
    /// crossing, so non-monotone TVD (horizontal / build-up) is handled.
    pub fn md_at_tvd(&self, tvd: f64) -> Option<f64> {
        for w in self.nodes.windows(2) {
            let (a, b) = (w[0], w[1]);
            let (z0, z1) = (a.p.z, b.p.z);
            let within = (tvd >= z0 && tvd <= z1) || (tvd <= z0 && tvd >= z1);
            if within {
                let dz = z1 - z0;
                if dz == 0.0 {
                    return Some(a.md);
                }
                return Some(a.md + (tvd - z0) / dz * (b.md - a.md));
            }
        }
        self.nodes.first().filter(|n| n.p.z == tvd).map(|n| n.md)
    }
}

/// Minimum-curvature normalization of a station list. See module docs for the
/// depth convention.
fn min_curvature(stations: &[Station], head: (f64, f64), kb: f64) -> Result<Vec<Node>> {
    let s0 = *stations
        .first()
        .ok_or_else(|| GeoError::OutOfRange("trajectory needs at least one station".into()))?;
    let mut north = 0.0_f64;
    let mut east = 0.0_f64;
    let mut tvd = s0.md - kb;
    let mut nodes = Vec::with_capacity(stations.len());
    nodes.push(Node {
        md: s0.md,
        p: Point3::new(head.0 + east, head.1 + north, tvd),
        dir: Some(tangent(s0.inc_deg, s0.azi_deg)),
    });
    for w in stations.windows(2) {
        let (a, b) = (w[0], w[1]);
        let dmd = b.md - a.md;
        if dmd <= 0.0 {
            return Err(GeoError::OutOfRange(
                "station measured depth must strictly increase".into(),
            ));
        }
        let (i1, i2) = (a.inc_deg.to_radians(), b.inc_deg.to_radians());
        let (a1, a2) = (a.azi_deg.to_radians(), b.azi_deg.to_radians());
        let cos_b = (i2 - i1).cos() - i1.sin() * i2.sin() * (1.0 - (a2 - a1).cos());
        let beta = cos_b.clamp(-1.0, 1.0).acos();
        let rf = if beta < SMALL_BETA {
            1.0 + beta * beta / 12.0
        } else {
            (2.0 / beta) * (beta / 2.0).tan()
        };
        let half = 0.5 * dmd * rf;
        north += half * (i1.sin() * a1.cos() + i2.sin() * a2.cos());
        east += half * (i1.sin() * a1.sin() + i2.sin() * a2.sin());
        tvd += half * (i1.cos() + i2.cos());
        nodes.push(Node {
            md: b.md,
            p: Point3::new(head.0 + east, head.1 + north, tvd),
            dir: Some(tangent(b.inc_deg, b.azi_deg)),
        });
    }
    Ok(nodes)
}

/// Unit tangent `[north, east, down]` for a station's inclination/azimuth.
fn tangent(inc_deg: f64, azi_deg: f64) -> [f64; 3] {
    let (i, a) = (inc_deg.to_radians(), azi_deg.to_radians());
    [i.sin() * a.cos(), i.sin() * a.sin(), i.cos()]
}

/// Position at fraction `f ∈ [0, 1]` of the minimum-curvature arc from station
/// `pa` (tangent `t1`) to the next station (tangent `t2`) over MD span `dmd`.
/// The tangent at `f` is the slerp of `t1`/`t2`; the partial dogleg is `f·β`.
/// At `f = 1` this reproduces the next station's stored position exactly.
fn arc_point(pa: Point3, t1: [f64; 3], t2: [f64; 3], f: f64, dmd: f64) -> Point3 {
    let dot = (t1[0] * t2[0] + t1[1] * t2[1] + t1[2] * t2[2]).clamp(-1.0, 1.0);
    let beta = dot.acos();
    // Interpolated unit tangent at fraction f (slerp; lerp in the small-angle limit).
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
    let beta_f = f * beta;
    let rf = if beta_f < SMALL_BETA {
        1.0 + beta_f * beta_f / 12.0
    } else {
        (2.0 / beta_f) * (beta_f / 2.0).tan()
    };
    let half = 0.5 * (f * dmd) * rf;
    // p.y ← north, p.x ← east, p.z ← down (matches min_curvature accumulation).
    Point3::new(
        pa.x + half * (t1[1] + tf[1]),
        pa.y + half * (t1[0] + tf[0]),
        pa.z + half * (t1[2] + tf[2]),
    )
}

/// Explicit positions → nodes; `md` is cumulative 3-D chord length from the
/// first point.
fn nodes_from_xyz(points: Vec<Point3>) -> Result<Vec<Node>> {
    let first = *points
        .first()
        .ok_or_else(|| GeoError::OutOfRange("trajectory needs at least one point".into()))?;
    let mut nodes = Vec::with_capacity(points.len());
    let mut md = 0.0;
    let mut prev = first;
    nodes.push(Node {
        md,
        p: first,
        dir: None,
    });
    for p in points.into_iter().skip(1) {
        md += dist3(prev, p);
        nodes.push(Node { md, p, dir: None });
        prev = p;
    }
    Ok(nodes)
}

/// Sample a `Steer` segment into stations at a fixed MD step (build/turn rates
/// are degrees per 100 MD, linear in MD).
fn steer_stations(
    from: Station,
    build_per_100: f64,
    turn_per_100: f64,
    to_md: f64,
) -> Vec<Station> {
    let at = |md: f64| {
        let d = md - from.md;
        Station::new(
            md,
            from.inc_deg + build_per_100 * d / 100.0,
            from.azi_deg + turn_per_100 * d / 100.0,
        )
    };
    let mut out = vec![from];
    let mut md = from.md + STEER_STEP;
    while md < to_md - 1e-9 {
        out.push(at(md));
        md += STEER_STEP;
    }
    out.push(at(to_md));
    out
}

/// Linear interpolation between two points at parameter `t ∈ [0, 1]`.
fn lerp3(a: Point3, b: Point3, t: f64) -> Point3 {
    Point3::new(
        a.x + (b.x - a.x) * t,
        a.y + (b.y - a.y) * t,
        a.z + (b.z - a.z) * t,
    )
}

/// Euclidean 3-D distance between two points.
fn dist3(a: Point3, b: Point3) -> f64 {
    ((b.x - a.x).powi(2) + (b.y - a.y).powi(2) + (b.z - a.z).powi(2)).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    fn traj(input: TrajectoryInput, head: (f64, f64), kb: f64) -> Trajectory {
        Trajectory::from_input(input, head, kb).unwrap()
    }

    #[test]
    fn golden_min_curvature_survey() {
        // Hand-verified worked survey from dev-docs/plans/wells.md §A.
        let t = traj(
            TrajectoryInput::MdIncAzi(vec![
                Station::new(3500.0, 15.0, 20.0),
                Station::new(3600.0, 25.0, 45.0),
            ]),
            (1000.0, 2000.0),
            100.0,
        );
        let p0 = t.xyz(3500.0).unwrap();
        assert_relative_eq!(p0.x, 1000.0, epsilon = 1e-9);
        assert_relative_eq!(p0.y, 2000.0, epsilon = 1e-9);
        assert_relative_eq!(p0.z, 3400.0, epsilon = 1e-9); // 3500 - kb

        let p = t.xyz(3600.0).unwrap();
        // ΔN ≈ 27.216 (y/Northing), ΔE ≈ 19.449 (x/Easting), ΔTVD ≈ 94.005.
        assert_relative_eq!(p.x, 1000.0 + 19.449, epsilon = 0.01);
        assert_relative_eq!(p.y, 2000.0 + 27.216, epsilon = 0.01);
        assert_relative_eq!(p.z, 3400.0 + 94.005, epsilon = 0.01);
    }

    #[test]
    fn vertical_well_degenerate() {
        let t = traj(
            TrajectoryInput::Stations(vec![
                Station::new(0.0, 0.0, 0.0),
                Station::new(1000.0, 0.0, 0.0),
                Station::new(2000.0, 0.0, 0.0),
            ]),
            (500.0, 600.0),
            30.0,
        );
        for md in [0.0, 750.0, 1000.0, 1500.0, 2000.0] {
            let p = t.xyz(md).unwrap();
            assert_relative_eq!(p.x, 500.0, epsilon = 1e-9);
            assert_relative_eq!(p.y, 600.0, epsilon = 1e-9);
            assert_relative_eq!(p.z, md - 30.0, epsilon = 1e-9); // tvd = md - kb
            assert_relative_eq!(t.tvd(md).unwrap(), md - 30.0, epsilon = 1e-9);
        }
    }

    #[test]
    fn outside_md_range_is_none() {
        let t = traj(
            TrajectoryInput::Stations(vec![
                Station::new(100.0, 0.0, 0.0),
                Station::new(200.0, 0.0, 0.0),
            ]),
            (0.0, 0.0),
            0.0,
        );
        assert_eq!(t.md_range(), (100.0, 200.0));
        assert!(t.xyz(99.0).is_none());
        assert!(t.xyz(201.0).is_none());
        assert!(t.xyz(150.0).is_some());
    }

    #[test]
    fn xyz_follows_min_curvature_arc_between_stations() {
        let t = traj(
            TrajectoryInput::MdIncAzi(vec![
                Station::new(3500.0, 15.0, 20.0),
                Station::new(3600.0, 25.0, 45.0),
            ]),
            (0.0, 0.0),
            0.0,
        );
        let a = t.xyz(3500.0).unwrap();
        let b = t.xyz(3600.0).unwrap();
        let mid = t.xyz(3550.0).unwrap();
        // The arc bows off the straight chord between stations — it is NOT the
        // linear midpoint (that was the pre-fix behaviour).
        let (cx, cy, cz) = ((a.x + b.x) / 2.0, (a.y + b.y) / 2.0, (a.z + b.z) / 2.0);
        let dev = ((mid.x - cx).powi(2) + (mid.y - cy).powi(2) + (mid.z - cz).powi(2)).sqrt();
        assert!(
            dev > 1e-3,
            "arc midpoint should depart from the chord (dev={dev})"
        );
        // Endpoints still reproduce the stored station nodes exactly.
        assert_relative_eq!(t.xyz(3500.0).unwrap().z, a.z, epsilon = 1e-12);
        assert_relative_eq!(t.xyz(3600.0).unwrap().z, b.z, epsilon = 1e-12);
        // MD spacing along the arc is monotone in TVD here (building, TVD increases).
        assert!(a.z < mid.z && mid.z < b.z);
    }

    #[test]
    fn md_at_tvd_on_build_up_path() {
        // Vertical then building to 30° — TVD monotone increasing.
        let t = traj(
            TrajectoryInput::Stations(vec![
                Station::new(0.0, 0.0, 0.0),
                Station::new(1000.0, 0.0, 0.0),
                Station::new(2000.0, 30.0, 0.0),
            ]),
            (0.0, 0.0),
            0.0,
        );
        let (lo, hi) = t.md_range();
        let target = (t.tvd(lo).unwrap() + t.tvd(hi).unwrap()) / 2.0;
        let md = t.md_at_tvd(target).unwrap();
        assert!(md >= lo && md <= hi);
        assert_relative_eq!(t.tvd(md).unwrap(), target, epsilon = 1e-9);
    }

    #[test]
    fn hold_is_a_straight_constant_segment() {
        // 30° inclination due East (azimuth 90°) over 1000 MD.
        let t = traj(
            TrajectoryInput::Hold {
                from: Station::new(0.0, 30.0, 90.0),
                to_md: 1000.0,
            },
            (0.0, 0.0),
            0.0,
        );
        let p = t.xyz(1000.0).unwrap();
        assert_relative_eq!(p.x, 500.0, epsilon = 1e-6); // sin30° · 1000 East
        assert_relative_eq!(p.y, 0.0, epsilon = 1e-9); // no Northing
        assert_relative_eq!(p.z, 30.0_f64.to_radians().cos() * 1000.0, epsilon = 1e-6);
    }

    #[test]
    fn xyz_input_uses_positions_directly() {
        let t = traj(
            TrajectoryInput::Xyz(vec![
                Point3::new(0.0, 0.0, 0.0),
                Point3::new(0.0, 0.0, 100.0),
            ]),
            (999.0, 999.0),
            999.0,
        );
        assert_eq!(t.md_range(), (0.0, 100.0));
        let p = t.xyz(50.0).unwrap();
        assert_relative_eq!(p.x, 0.0, epsilon = 1e-9);
        assert_relative_eq!(p.z, 50.0, epsilon = 1e-9);
    }

    #[test]
    fn steer_builds_inclination() {
        // Build 3°/100 from vertical over 1000 MD → ends at 30° inclination.
        let t = traj(
            TrajectoryInput::Steer {
                from: Station::new(0.0, 0.0, 0.0),
                build_per_100: 3.0,
                turn_per_100: 0.0,
                to_md: 1000.0,
            },
            (0.0, 0.0),
            0.0,
        );
        let (_, hi) = t.md_range();
        assert_relative_eq!(hi, 1000.0, epsilon = 1e-9);
        // Some horizontal departure was built; TVD < MD.
        let p = t.xyz(hi).unwrap();
        assert!(p.x.hypot(p.y) > 0.0);
        assert!(p.z < 1000.0);
    }

    #[test]
    fn non_increasing_md_errors() {
        let r = Trajectory::from_input(
            TrajectoryInput::Stations(vec![
                Station::new(100.0, 0.0, 0.0),
                Station::new(100.0, 0.0, 0.0),
            ]),
            (0.0, 0.0),
            0.0,
        );
        assert!(r.is_err());
    }
}
