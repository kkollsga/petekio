//! Regular-lattice fitting — the **downward** conversions' engine.
//!
//! Two fitters, one per evidence kind, both strict (they *refuse* rather than
//! return a lattice no node sits on):
//!
//! * [`fit_grid_from_indexed`] — nodes carrying `(column, row)` indices. Axes
//!   and spacing come from median inter-index steps; the fit is confirmed
//!   against the **median** node residual (isolated off-lattice nodes are an
//!   expected export artefact; a mostly-off-lattice mesh is curvilinear and has
//!   no `GridGeometry` at all).
//! * [`fit_grid_from_coords`] — bare XY nodes. Axes/spacing are detected from
//!   nearest-neighbour vectors and **every** node must land on the lattice
//!   within `tolerance`.
//!
//! Callers: `PointSet::infer_geometry*` (via `core::points`),
//! [`StructuredShell::infer_grid`](super::StructuredShell::infer_grid) and
//! [`MeshShell::infer_grid`](super::MeshShell::infer_grid).

use crate::core::points::AerialEntry;
use crate::foundation::{GeoError, GridGeometry, Result};
use rstar::primitives::GeomWithData;
use rstar::RTree;

/// Fit a regular [`GridGeometry`] to `(column, row, x, y)`-indexed nodes.
///
/// Spacing/rotation/origin are medians over adjacent indexed nodes; the fit is
/// accepted only when the **median** node residual is within `tolerance`.
pub(crate) fn fit_grid_from_indexed(
    indexed: &[(isize, isize, f64, f64)],
    tolerance: f64,
) -> Result<GridGeometry> {
    if !tolerance.is_finite() || tolerance <= 0.0 {
        return Err(GeoError::GeometryInference(
            "tolerance must be a finite positive number".into(),
        ));
    }
    if indexed.len() < 4 {
        return Err(GeoError::GeometryInference(
            "column/row geometry inference requires at least four indexed points".into(),
        ));
    }

    let min_col = indexed.iter().map(|p| p.0).min().unwrap();
    let max_col = indexed.iter().map(|p| p.0).max().unwrap();
    let min_row = indexed.iter().map(|p| p.1).min().unwrap();
    let max_row = indexed.iter().map(|p| p.1).max().unwrap();
    if max_col <= min_col || max_row <= min_row {
        return Err(GeoError::GeometryInference(
            "column/row attributes do not span a two-dimensional grid".into(),
        ));
    }

    let mut by_index = std::collections::BTreeMap::new();
    for (col, row, x, y) in indexed {
        by_index.entry((*col, *row)).or_insert((*x, *y));
    }

    let mut i_dx = Vec::new();
    let mut i_dy = Vec::new();
    let mut j_dx = Vec::new();
    let mut j_dy = Vec::new();
    for ((col, row), (x, y)) in &by_index {
        if let Some((nx, ny)) = by_index.get(&(*col + 1, *row)) {
            let dx = nx - x;
            let dy = ny - y;
            if dx.hypot(dy) > tolerance {
                i_dx.push(dx);
                i_dy.push(dy);
            }
        }
        if let Some((nx, ny)) = by_index.get(&(*col, *row + 1)) {
            let dx = nx - x;
            let dy = ny - y;
            if dx.hypot(dy) > tolerance {
                j_dx.push(dx);
                j_dy.push(dy);
            }
        }
    }

    let (Some(m_i_dx), Some(m_i_dy), Some(m_j_dx), Some(m_j_dy)) = (
        median_unsorted(&i_dx),
        median_unsorted(&i_dy),
        median_unsorted(&j_dx),
        median_unsorted(&j_dy),
    ) else {
        return Err(GeoError::GeometryInference(
            "column/row attributes are present, but adjacent indexed nodes are too sparse to infer spacing".into(),
        ));
    };

    let e1 = unit([m_i_dx, m_i_dy])?;
    let xinc = m_i_dx.hypot(m_i_dy);
    let perp = [-e1[1], e1[0]];
    let row_projection = m_j_dx * perp[0] + m_j_dy * perp[1];
    if row_projection.abs() <= tolerance {
        return Err(GeoError::GeometryInference(
            "column/row attributes imply a degenerate row spacing".into(),
        ));
    }
    let yinc = row_projection.abs();
    let yflip = row_projection < 0.0;
    let ysign = if yflip { -1.0 } else { 1.0 };

    let mut origins_x = Vec::with_capacity(indexed.len());
    let mut origins_y = Vec::with_capacity(indexed.len());
    for (col, row, x, y) in indexed {
        let i = (col - min_col) as f64;
        let j = (row - min_row) as f64;
        origins_x.push(x - i * xinc * e1[0] - j * yinc * ysign * perp[0]);
        origins_y.push(y - i * xinc * e1[1] - j * yinc * ysign * perp[1]);
    }
    let xori = median_unsorted(&origins_x)
        .ok_or_else(|| GeoError::GeometryInference("could not infer indexed grid origin".into()))?;
    let yori = median_unsorted(&origins_y)
        .ok_or_else(|| GeoError::GeometryInference("could not infer indexed grid origin".into()))?;

    let geom = GridGeometry {
        xori,
        yori,
        xinc,
        yinc,
        ncol: (max_col - min_col + 1) as usize,
        nrow: (max_row - min_row + 1) as usize,
        rotation_deg: e1[1].atan2(e1[0]).to_degrees(),
        yflip,
    };

    // Spacing, rotation and origin above are *medians* over the indexed nodes, so a
    // curvilinear or locally warped mesh still yields a plausible-looking regular
    // lattice that no node actually sits on. Confirm the lattice really describes the
    // mesh before handing it back, or callers get a silently wrong geometry.
    //
    // The test is the *median* node residual, not the max: isolated nodes that miss
    // the lattice are an expected export artefact (Petrel collapses or clips single
    // nodes, and topology is meant to win over those — see
    // `infer_geometry_uses_explicit_column_row_topology`). A mesh whose nodes are
    // mostly off-lattice is not a regular grid at all, and no GridGeometry can
    // describe it.
    let residuals: Vec<f64> = indexed
        .iter()
        .map(|(col, row, x, y)| {
            let (nx, ny) = geom.node_xy((col - min_col) as usize, (row - min_row) as usize);
            (x - nx).hypot(y - ny)
        })
        .collect();
    let median_residual = median_unsorted(&residuals).unwrap_or(0.0);
    if median_residual > tolerance {
        let worst = residuals.iter().copied().fold(0.0_f64, f64::max);
        return Err(GeoError::GeometryInference(format!(
            "column/row nodes miss the inferred regular lattice by a median of \
             {median_residual:.6} (worst {worst:.6}), above tolerance {tolerance:.6}; the mesh \
             is curvilinear rather than a regular grid — use to_structured_surface, which \
             carries explicit per-node XY"
        )));
    }

    Ok(geom)
}

/// Fit a regular [`GridGeometry`] to bare coordinates, also returning the
/// occupied `(i, j)` lattice indices. Strict: every finite point must land on
/// the inferred lattice within `tolerance`, no node may be claimed twice.
pub(crate) fn fit_grid_from_coords(
    coords: &[[f64; 3]],
    tolerance: f64,
) -> Result<(GridGeometry, Vec<(usize, usize)>)> {
    if !tolerance.is_finite() || tolerance <= 0.0 {
        return Err(GeoError::GeometryInference(
            "tolerance must be a finite positive number".into(),
        ));
    }

    let pts: Vec<[f64; 2]> = coords
        .iter()
        .filter(|c| c[0].is_finite() && c[1].is_finite())
        .map(|c| [c[0], c[1]])
        .collect();
    if pts.len() < 4 {
        return Err(GeoError::GeometryInference(
            "at least four finite points are required".into(),
        ));
    }

    let vectors = neighbour_vectors(&pts, tolerance);
    if vectors.len() < 2 {
        return Err(GeoError::GeometryInference(
            "not enough neighbouring points to detect grid axes".into(),
        ));
    }

    let (e1, e2, xinc, yinc) = infer_axes_and_spacing(&vectors, tolerance)?;
    let anchor = pts[0];
    let mut uv: Vec<(f64, f64)> = Vec::with_capacity(pts.len());
    for p in &pts {
        let dx = p[0] - anchor[0];
        let dy = p[1] - anchor[1];
        uv.push((dx * e1[0] + dy * e1[1], dx * e2[0] + dy * e2[1]));
    }

    let min_u = uv.iter().map(|p| p.0).fold(f64::INFINITY, f64::min);
    let min_v = uv.iter().map(|p| p.1).fold(f64::INFINITY, f64::min);
    let mut ij: Vec<(isize, isize)> = Vec::with_capacity(uv.len());
    let mut max_i = 0isize;
    let mut max_j = 0isize;
    let mut max_residual = 0.0_f64;

    for (u, v) in uv {
        let fi = (u - min_u) / xinc;
        let fj = (v - min_v) / yinc;
        let i = fi.round() as isize;
        let j = fj.round() as isize;
        if i < 0 || j < 0 {
            return Err(GeoError::GeometryInference(
                "inferred negative lattice index; grid origin is ambiguous".into(),
            ));
        }
        let du = (fi - i as f64).abs() * xinc;
        let dv = (fj - j as f64).abs() * yinc;
        let residual = du.hypot(dv);
        max_residual = max_residual.max(residual);
        if residual > tolerance {
            return Err(GeoError::GeometryInference(format!(
                "point misses inferred lattice by {residual:.6}, above tolerance {tolerance:.6}"
            )));
        }
        max_i = max_i.max(i);
        max_j = max_j.max(j);
        ij.push((i, j));
    }

    ij.sort_unstable();
    if ij.windows(2).any(|w| w[0] == w[1]) {
        return Err(GeoError::GeometryInference(
            "multiple points map to the same inferred grid node".into(),
        ));
    }
    if max_i < 1 || max_j < 1 {
        return Err(GeoError::GeometryInference(
            "detected points do not span a two-dimensional grid".into(),
        ));
    }

    let xori = anchor[0] + min_u * e1[0] + min_v * e2[0];
    let yori = anchor[1] + min_u * e1[1] + min_v * e2[1];
    let rotation_deg = e1[1].atan2(e1[0]).to_degrees();

    let geom = GridGeometry {
        xori,
        yori,
        xinc,
        yinc,
        ncol: (max_i + 1) as usize,
        nrow: (max_j + 1) as usize,
        rotation_deg,
        yflip: false,
    };

    if max_residual > tolerance {
        return Err(GeoError::GeometryInference(format!(
            "maximum lattice residual {max_residual:.6} exceeds tolerance {tolerance:.6}"
        )));
    }
    let occupancy = ij
        .into_iter()
        .map(|(i, j)| (i as usize, j as usize))
        .collect();
    Ok((geom, occupancy))
}

fn neighbour_vectors(pts: &[[f64; 2]], tolerance: f64) -> Vec<[f64; 2]> {
    let entries: Vec<AerialEntry> = pts
        .iter()
        .enumerate()
        .map(|(i, p)| GeomWithData::new(*p, i))
        .collect();
    let tree = RTree::bulk_load(entries);
    let stride = (pts.len() / 2000).max(1);
    let mut vectors = Vec::new();

    for (idx, p) in pts.iter().enumerate().step_by(stride) {
        for neighbour in tree.nearest_neighbor_iter(*p).take(13) {
            if neighbour.data == idx {
                continue;
            }
            let q = pts[neighbour.data];
            let dx = q[0] - p[0];
            let dy = q[1] - p[1];
            if dx.hypot(dy) > tolerance {
                vectors.push([dx, dy]);
            }
        }
    }
    vectors
}

fn infer_axes_and_spacing(
    vectors: &[[f64; 2]],
    tolerance: f64,
) -> Result<([f64; 2], [f64; 2], f64, f64)> {
    let mut by_len: Vec<[f64; 2]> = vectors.to_vec();
    by_len.sort_by(|a, b| a[0].hypot(a[1]).total_cmp(&b[0].hypot(b[1])));

    let first = *by_len
        .first()
        .ok_or_else(|| GeoError::GeometryInference("no neighbour vectors found".into()))?;
    let mut e1 = unit(first)?;
    if e1[0] < 0.0 || (e1[0].abs() <= f64::EPSILON && e1[1] < 0.0) {
        e1 = [-e1[0], -e1[1]];
    }
    let e2 = [-e1[1], e1[0]];

    let xinc = spacing_along(vectors, e1, tolerance)?;
    let yinc = spacing_along(vectors, e2, tolerance)?;
    Ok((e1, e2, xinc, yinc))
}

fn unit(v: [f64; 2]) -> Result<[f64; 2]> {
    let d = v[0].hypot(v[1]);
    if d == 0.0 || !d.is_finite() {
        return Err(GeoError::GeometryInference(
            "zero-length vector while detecting grid axes".into(),
        ));
    }
    Ok([v[0] / d, v[1] / d])
}

fn spacing_along(vectors: &[[f64; 2]], axis: [f64; 2], tolerance: f64) -> Result<f64> {
    let mut projected: Vec<f64> = vectors
        .iter()
        .filter_map(|v| {
            let along = (v[0] * axis[0] + v[1] * axis[1]).abs();
            let across = (v[0] * -axis[1] + v[1] * axis[0]).abs();
            (along > tolerance && across <= tolerance).then_some(along)
        })
        .collect();
    projected.sort_by(|a, b| a.total_cmp(b));
    let shortest = projected.first().copied().ok_or_else(|| {
        GeoError::GeometryInference("could not detect regular spacing on both grid axes".into())
    })?;
    let near_step: Vec<f64> = projected
        .into_iter()
        .filter(|v| *v <= shortest * 1.5 + tolerance)
        .collect();
    median(&near_step).ok_or_else(|| {
        GeoError::GeometryInference("could not detect regular spacing on both grid axes".into())
    })
}

fn median(values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let mid = values.len() / 2;
    if values.len().is_multiple_of(2) {
        Some((values[mid - 1] + values[mid]) * 0.5)
    } else {
        Some(values[mid])
    }
}

pub(crate) fn median_unsorted(values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.total_cmp(b));
    median(&sorted)
}
