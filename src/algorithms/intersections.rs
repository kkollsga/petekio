//! Type-light curve/triangle-surface intersection kernels.
//!
//! Domain objects marshal their trajectories and surface shells into this
//! module.  The numerical work has one home: adaptive curve subdivision,
//! spatial triangle candidate lookup, line/triangle tests, tangent probes,
//! root refinement, coplanar rejection, and shared-edge de-duplication.

use crate::foundation::{GeoError, Point3, Result};
use rstar::{RTree, RTreeObject, AABB};

/// One numerical curve/surface hit before domain identity is attached.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CurveHit {
    pub md: f64,
    pub xyz: Point3,
}

#[derive(Clone)]
struct IndexedTriangle {
    index: usize,
    envelope: AABB<[f64; 3]>,
}

impl RTreeObject for IndexedTriangle {
    type Envelope = AABB<[f64; 3]>;

    fn envelope(&self) -> Self::Envelope {
        self.envelope
    }
}

/// Intersect an MD-parameterised curve with a triangle surface.
///
/// `breaks` are the original survey-station MDs. Each interval is subdivided
/// until both its chord length and curvature error are small relative to the
/// surface mesh, so the callback continues to evaluate the actual trajectory
/// (including minimum-curvature arcs) rather than a station polyline.
pub fn intersect_curve_surface(
    breaks: &[f64],
    position: impl Fn(f64) -> Option<Point3>,
    vertices: &[Point3],
    triangles: &[[u32; 3]],
    tolerance: f64,
) -> Result<Vec<CurveHit>> {
    if !tolerance.is_finite() || tolerance <= 0.0 {
        return Err(GeoError::OutOfRange(
            "intersection tolerance must be finite and positive".into(),
        ));
    }
    if breaks.len() < 2 {
        return Ok(Vec::new());
    }

    let valid = valid_triangles(vertices, triangles);
    if valid.is_empty() {
        return Ok(Vec::new());
    }
    let scale = mesh_scale(vertices, triangles, &valid).max(tolerance * 8.0);
    let tree = RTree::bulk_load(
        valid
            .iter()
            .map(|&index| IndexedTriangle {
                index,
                envelope: triangle_envelope(vertices, triangles[index], tolerance),
            })
            .collect(),
    );

    let mut segments = Vec::new();
    for pair in breaks.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        let (Some(pa), Some(pb)) = (position(a), position(b)) else {
            continue;
        };
        subdivide_curve(a, pa, b, pb, 0, scale, tolerance, &position, &mut segments);
    }

    let mut hits = Vec::new();
    for (md0, p0, md1, p1) in segments {
        let env = segment_envelope(p0, p1, tolerance);
        for item in tree.locate_in_envelope_intersecting(env) {
            let tri = triangles[item.index];
            let [a, b, c] = triangle_points(vertices, tri);
            if segment_coplanar_overlap(p0, p1, a, b, c, tolerance) {
                return Err(GeoError::Unsupported(format!(
                    "trajectory interval {md0:.6}..{md1:.6} is coplanar with the surface; intersection is not a discrete pick"
                )));
            }
            if let Some(t) = segment_triangle(p0, p1, a, b, c, tolerance) {
                let seed = md0 + t * (md1 - md0);
                let md = refine_plane_root(seed, md0, md1, a, b, c, tolerance, &position);
                if let Some(xyz) = position(md) {
                    if point_in_triangle(xyz, a, b, c, tolerance) {
                        hits.push(CurveHit { md, xyz });
                    }
                }
            } else if let Some(hit) = tangent_probe(md0, md1, a, b, c, tolerance, &position) {
                hits.push(hit);
            }
        }
    }

    hits.sort_by(|a, b| a.md.total_cmp(&b.md));
    // The same geometric crossing is commonly emitted by both triangles on a
    // quad diagonal and by the two adaptive chords adjacent to a subdivision
    // boundary. Root refinement can place those a few √tol apart in MD even
    // though their XYZ is indistinguishable.
    let md_tol = tolerance.sqrt().max(tolerance) * 4.0;
    hits.dedup_by(|b, a| {
        (b.md - a.md).abs() <= md_tol && distance(b.xyz, a.xyz) <= tolerance * 4.0
    });
    Ok(hits)
}

#[allow(clippy::too_many_arguments)]
fn subdivide_curve(
    md0: f64,
    p0: Point3,
    md1: f64,
    p1: Point3,
    depth: u8,
    scale: f64,
    tolerance: f64,
    position: &impl Fn(f64) -> Option<Point3>,
    out: &mut Vec<(f64, Point3, f64, Point3)>,
) {
    const MAX_DEPTH: u8 = 24;
    let mid = 0.5 * (md0 + md1);
    let Some(pm) = position(mid) else {
        out.push((md0, p0, md1, p1));
        return;
    };
    let chord_mid = lerp(p0, p1, 0.5);
    let curved = distance(pm, chord_mid) > tolerance * 0.25;
    let long = distance(p0, p1) > scale * 0.25;
    if depth < MAX_DEPTH && (curved || long) && md1 - md0 > tolerance * 0.25 {
        subdivide_curve(md0, p0, mid, pm, depth + 1, scale, tolerance, position, out);
        subdivide_curve(mid, pm, md1, p1, depth + 1, scale, tolerance, position, out);
    } else {
        out.push((md0, p0, md1, p1));
    }
}

fn valid_triangles(vertices: &[Point3], triangles: &[[u32; 3]]) -> Vec<usize> {
    triangles
        .iter()
        .enumerate()
        .filter_map(|(i, t)| {
            let ids = [t[0] as usize, t[1] as usize, t[2] as usize];
            let finite = ids.iter().all(|&j| {
                vertices
                    .get(j)
                    .is_some_and(|p| p.x.is_finite() && p.y.is_finite() && p.z.is_finite())
            });
            finite.then_some(i)
        })
        .collect()
}

fn mesh_scale(vertices: &[Point3], triangles: &[[u32; 3]], valid: &[usize]) -> f64 {
    let mut edges = Vec::with_capacity(valid.len() * 3);
    for &i in valid {
        let [a, b, c] = triangle_points(vertices, triangles[i]);
        edges.extend([distance(a, b), distance(b, c), distance(c, a)]);
    }
    edges.retain(|v| v.is_finite() && *v > 0.0);
    edges.sort_by(f64::total_cmp);
    edges.get(edges.len() / 2).copied().unwrap_or(1.0)
}

fn triangle_points(vertices: &[Point3], t: [u32; 3]) -> [Point3; 3] {
    [
        vertices[t[0] as usize],
        vertices[t[1] as usize],
        vertices[t[2] as usize],
    ]
}

fn triangle_envelope(vertices: &[Point3], t: [u32; 3], pad: f64) -> AABB<[f64; 3]> {
    let p = triangle_points(vertices, t);
    envelope(&p, pad)
}

fn segment_envelope(a: Point3, b: Point3, pad: f64) -> AABB<[f64; 3]> {
    envelope(&[a, b], pad)
}

fn envelope(points: &[Point3], pad: f64) -> AABB<[f64; 3]> {
    let lo = [
        points.iter().map(|p| p.x).fold(f64::INFINITY, f64::min) - pad,
        points.iter().map(|p| p.y).fold(f64::INFINITY, f64::min) - pad,
        points.iter().map(|p| p.z).fold(f64::INFINITY, f64::min) - pad,
    ];
    let hi = [
        points.iter().map(|p| p.x).fold(f64::NEG_INFINITY, f64::max) + pad,
        points.iter().map(|p| p.y).fold(f64::NEG_INFINITY, f64::max) + pad,
        points.iter().map(|p| p.z).fold(f64::NEG_INFINITY, f64::max) + pad,
    ];
    AABB::from_corners(lo, hi)
}

/// Möller–Trumbore segment/triangle intersection. Returns segment fraction.
fn segment_triangle(
    p0: Point3,
    p1: Point3,
    a: Point3,
    b: Point3,
    c: Point3,
    tol: f64,
) -> Option<f64> {
    let d = sub(p1, p0);
    let e1 = sub(b, a);
    let e2 = sub(c, a);
    let h = cross(d, e2);
    let det = dot(e1, h);
    let eps = tol * norm(e1).max(norm(e2)).max(1.0);
    if det.abs() <= eps {
        return None;
    }
    let inv = 1.0 / det;
    let s = sub(p0, a);
    let u = inv * dot(s, h);
    if u < -tol || u > 1.0 + tol {
        return None;
    }
    let q = cross(s, e1);
    let v = inv * dot(d, q);
    if v < -tol || u + v > 1.0 + tol {
        return None;
    }
    let t = inv * dot(e2, q);
    (t >= -tol && t <= 1.0 + tol).then(|| t.clamp(0.0, 1.0))
}

fn tangent_probe(
    md0: f64,
    md1: f64,
    a: Point3,
    b: Point3,
    c: Point3,
    tol: f64,
    position: &impl Fn(f64) -> Option<Point3>,
) -> Option<CurveHit> {
    let mut best: Option<(f64, Point3, f64)> = None;
    for i in 0..=4 {
        let md = md0 + (md1 - md0) * (i as f64 / 4.0);
        let p = position(md)?;
        if !point_in_triangle(p, a, b, c, tol) {
            continue;
        }
        let d = plane_distance(p, a, b, c).abs();
        if best.is_none_or(|(_, _, old)| d < old) {
            best = Some((md, p, d));
        }
    }
    let (seed, _, d) = best?;
    if d > tol {
        return None;
    }
    let md = minimize_plane_distance(seed, md0, md1, a, b, c, position);
    let xyz = position(md)?;
    (plane_distance(xyz, a, b, c).abs() <= tol && point_in_triangle(xyz, a, b, c, tol))
        .then_some(CurveHit { md, xyz })
}

fn segment_coplanar_overlap(
    p0: Point3,
    p1: Point3,
    a: Point3,
    b: Point3,
    c: Point3,
    tol: f64,
) -> bool {
    let mid = lerp(p0, p1, 0.5);
    [p0, mid, p1]
        .iter()
        .all(|&p| plane_distance(p, a, b, c).abs() <= tol)
        && [p0, mid, p1]
            .iter()
            .any(|&p| point_in_triangle(p, a, b, c, tol))
        && distance(p0, p1) > tol
}

#[allow(clippy::too_many_arguments)]
fn refine_plane_root(
    seed: f64,
    mut lo: f64,
    mut hi: f64,
    a: Point3,
    b: Point3,
    c: Point3,
    tol: f64,
    position: &impl Fn(f64) -> Option<Point3>,
) -> f64 {
    let mut flo = position(lo)
        .map(|p| plane_distance(p, a, b, c))
        .unwrap_or(0.0);
    let fhi = position(hi)
        .map(|p| plane_distance(p, a, b, c))
        .unwrap_or(0.0);
    if flo.signum() == fhi.signum() {
        return minimize_plane_distance(seed, lo, hi, a, b, c, position);
    }
    for _ in 0..48 {
        let mid = 0.5 * (lo + hi);
        let Some(p) = position(mid) else { break };
        let fm = plane_distance(p, a, b, c);
        if fm.abs() <= tol * 0.1 || hi - lo <= tol * 0.1 {
            return mid;
        }
        if fm.signum() == flo.signum() {
            lo = mid;
            flo = fm;
        } else {
            hi = mid;
        }
    }
    0.5 * (lo + hi)
}

fn minimize_plane_distance(
    seed: f64,
    mut lo: f64,
    mut hi: f64,
    a: Point3,
    b: Point3,
    c: Point3,
    position: &impl Fn(f64) -> Option<Point3>,
) -> f64 {
    // Ternary minimisation is deterministic and sufficient on the already
    // short adaptive chord. Keep `seed` as a safe fallback.
    for _ in 0..36 {
        let x1 = lo + (hi - lo) / 3.0;
        let x2 = hi - (hi - lo) / 3.0;
        let d1 = position(x1)
            .map(|p| plane_distance(p, a, b, c).abs())
            .unwrap_or(f64::INFINITY);
        let d2 = position(x2)
            .map(|p| plane_distance(p, a, b, c).abs())
            .unwrap_or(f64::INFINITY);
        if d1 <= d2 {
            hi = x2;
        } else {
            lo = x1;
        }
    }
    let candidate = 0.5 * (lo + hi);
    if position(candidate).is_some() {
        candidate
    } else {
        seed
    }
}

fn point_in_triangle(p: Point3, a: Point3, b: Point3, c: Point3, tol: f64) -> bool {
    let v0 = sub(b, a);
    let v1 = sub(c, a);
    let v2 = sub(p, a);
    let d00 = dot(v0, v0);
    let d01 = dot(v0, v1);
    let d11 = dot(v1, v1);
    let d20 = dot(v2, v0);
    let d21 = dot(v2, v1);
    let denom = d00 * d11 - d01 * d01;
    if denom.abs() <= f64::EPSILON {
        return false;
    }
    let v = (d11 * d20 - d01 * d21) / denom;
    let w = (d00 * d21 - d01 * d20) / denom;
    let u = 1.0 - v - w;
    let eps = tol / norm(v0).max(norm(v1)).max(1.0);
    u >= -eps && v >= -eps && w >= -eps
}

fn plane_distance(p: Point3, a: Point3, b: Point3, c: Point3) -> f64 {
    let n = cross(sub(b, a), sub(c, a));
    let len = norm(n);
    if len == 0.0 {
        f64::INFINITY
    } else {
        dot(sub(p, a), n) / len
    }
}

fn lerp(a: Point3, b: Point3, t: f64) -> Point3 {
    Point3::new(
        a.x + t * (b.x - a.x),
        a.y + t * (b.y - a.y),
        a.z + t * (b.z - a.z),
    )
}

fn sub(a: Point3, b: Point3) -> Point3 {
    Point3::new(a.x - b.x, a.y - b.y, a.z - b.z)
}
fn dot(a: Point3, b: Point3) -> f64 {
    a.x * b.x + a.y * b.y + a.z * b.z
}
fn cross(a: Point3, b: Point3) -> Point3 {
    Point3::new(
        a.y * b.z - a.z * b.y,
        a.z * b.x - a.x * b.z,
        a.x * b.y - a.y * b.x,
    )
}
fn norm(a: Point3) -> f64 {
    dot(a, a).sqrt()
}
fn distance(a: Point3, b: Point3) -> f64 {
    norm(sub(a, b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vertical_curve_hits_shared_edge_once() {
        let vertices = vec![
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(1.0, 0.0, 0.0),
            Point3::new(1.0, 1.0, 0.0),
            Point3::new(0.0, 1.0, 0.0),
        ];
        let triangles = vec![[0, 1, 2], [0, 2, 3]];
        let hits = intersect_curve_surface(
            &[0.0, 2.0],
            |md| Some(Point3::new(0.5, 0.5, 1.0 - md)),
            &vertices,
            &triangles,
            1e-6,
        )
        .unwrap();
        assert_eq!(hits.len(), 1);
        assert!((hits[0].md - 1.0).abs() < 1e-5);
    }

    #[test]
    fn coplanar_curve_is_loud() {
        let vertices = vec![
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(1.0, 0.0, 0.0),
            Point3::new(0.0, 1.0, 0.0),
        ];
        let err = intersect_curve_surface(
            &[0.0, 1.0],
            |md| Some(Point3::new(md * 0.5, 0.25, 0.0)),
            &vertices,
            &[[0, 1, 2]],
            1e-6,
        )
        .unwrap_err();
        assert!(err.to_string().contains("coplanar"));
    }
}
