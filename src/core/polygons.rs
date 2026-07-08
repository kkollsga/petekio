//! `PolygonSet` — a collection of polygon rings (boundaries, faults, fluid
//! contacts) with point-in-polygon, area, bounding box, and surface clipping.
//! Backed by the `geo` crate's 2-D predicates. Imports from `foundation`, `io`,
//! and `core::surface`.

use crate::core::surface::Surface;
use crate::foundation::{BBox, GeoError, GridGeometry, HasHistory, OperationHistory, Result};
use crate::io::PolygonData;
use geo::prelude::*;
use geo::{Coord, LineString, Point, Polygon};
use indexmap::IndexMap;
use ndarray::Array2;
use std::path::Path;

/// A set of simple polygons (exterior rings, no holes). Z, if present in the
/// source, is dropped — all operations are areal. Undefined extents on an empty
/// set are `NaN`.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct PolygonSet {
    polys: Vec<Polygon<f64>>,
    #[serde(default)]
    attrs: IndexMap<String, Vec<f64>>,
    #[serde(default)]
    history: OperationHistory,
}

impl PolygonSet {
    /// Build from rings of `[x, y, z]` coordinates (Z ignored). Rings with
    /// fewer than three vertices are dropped (no area). `geo` closes each
    /// exterior ring automatically. Public: construct polygons in memory from
    /// coordinate arrays, without a file.
    pub fn from_rings(rings: Vec<Vec<[f64; 3]>>) -> PolygonSet {
        let polys = rings
            .into_iter()
            .filter(|r| r.len() >= 3)
            .map(|r| {
                let coords: Vec<Coord<f64>> =
                    r.iter().map(|c| Coord { x: c[0], y: c[1] }).collect();
                Polygon::new(LineString::new(coords), Vec::new())
            })
            .collect();
        PolygonSet {
            polys,
            attrs: IndexMap::new(),
            history: OperationHistory::from_entry("polygons.from_rings"),
        }
    }

    pub(crate) fn from_polygon_data(data: PolygonData) -> PolygonSet {
        PolygonSet::from_rings(data.into_rings())
    }

    /// Build the rectangular footprint of a grid geometry from its corner nodes.
    pub fn from_grid_geometry(geom: &GridGeometry) -> PolygonSet {
        let ni = geom.ncol.saturating_sub(1);
        let nj = geom.nrow.saturating_sub(1);
        let corners = [
            geom.node_xy(0, 0),
            geom.node_xy(ni, 0),
            geom.node_xy(ni, nj),
            geom.node_xy(0, nj),
        ];
        let ring = corners
            .into_iter()
            .map(|(x, y)| [x, y, 0.0])
            .collect::<Vec<_>>();
        let mut out = PolygonSet::from_rings(vec![ring]);
        out.history = OperationHistory::from_entry("polygons.from_grid_geometry");
        out
    }

    /// Convex hull of XY points as a polygon set. Returns `None` for degenerate
    /// inputs with fewer than three non-collinear points.
    pub fn convex_hull_xy(points: Vec<[f64; 2]>) -> Option<PolygonSet> {
        let hull = convex_hull(points);
        if hull.len() < 3 {
            return None;
        }
        let ring = hull.into_iter().map(|p| [p[0], p[1], 0.0]).collect();
        let mut out = PolygonSet::from_rings(vec![ring]);
        out.history = OperationHistory::from_entry("polygons.convex_hull_xy");
        Some(out)
    }

    /// Load polygons from a GeoJSON file (`Polygon`/`MultiPolygon`/`LineString`
    /// features; each ring becomes one polygon).
    pub fn load_geojson(path: impl AsRef<Path>) -> Result<PolygonSet> {
        let data = crate::io::vector::load_polygon_rings_geojson(path.as_ref())?;
        let mut out = PolygonSet::from_polygon_data(data);
        out.history = OperationHistory::from_entry(format!(
            "polygons.load_geojson(path={})",
            path.as_ref().display()
        ));
        Ok(out)
    }

    /// Load polygons from an IRAP/RMS plain `X Y Z` file (rings separated by the
    /// `999.0` sentinel).
    pub fn load_irap_polygons(path: impl AsRef<Path>) -> Result<PolygonSet> {
        let data = crate::io::xyz::load_polygons(path.as_ref())?;
        let mut out = PolygonSet::from_polygon_data(data);
        out.history = OperationHistory::from_entry(format!(
            "polygons.load_irap_polygons(path={})",
            path.as_ref().display()
        ));
        Ok(out)
    }

    /// Load polygons from a CPS-3 lines file (`.CPS3lines`) — polyline blocks
    /// each introduced by a `->` marker (see [`crate::io::cps3`]). Structure
    /// outlines, fault polygons, and model-edge rings.
    pub fn load_cps3_lines(path: impl AsRef<Path>) -> Result<PolygonSet> {
        let data = crate::io::cps3::load_cps3_lines(path.as_ref())?;
        let mut out = PolygonSet::from_polygon_data(data);
        out.history = OperationHistory::from_entry(format!(
            "polygons.load_cps3_lines(path={})",
            path.as_ref().display()
        ));
        Ok(out)
    }

    /// Load polygons from an ESRI shapefile (pass the `.shp` path).
    pub fn load_shapefile(path: impl AsRef<Path>) -> Result<PolygonSet> {
        let data = crate::io::vector::load_polygon_rings_shapefile(path.as_ref())?;
        let mut out = PolygonSet::from_polygon_data(data);
        out.history = OperationHistory::from_entry(format!(
            "polygons.load_shapefile(path={})",
            path.as_ref().display()
        ));
        Ok(out)
    }

    /// Whether `(x, y)` is inside any polygon. Uses `geo`'s `Contains`, which
    /// **excludes** the boundary (a point exactly on an edge is *not*
    /// contained).
    pub fn contains(&self, x: f64, y: f64) -> bool {
        let p = Point::new(x, y);
        self.polys.iter().any(|poly| poly.contains(&p))
    }

    /// Number of polygons in the set.
    pub fn len(&self) -> usize {
        self.polys.len()
    }

    /// Whether the set is empty.
    pub fn is_empty(&self) -> bool {
        self.polys.is_empty()
    }

    /// Total unsigned area of all polygons (summed; overlaps double-count).
    pub fn area(&self) -> f64 {
        self.polys.iter().map(|p| p.unsigned_area()).sum()
    }

    /// Unsigned area of each polygon, aligned 1:1 with this set's rows.
    pub fn area_values(&self) -> Vec<f64> {
        self.polys.iter().map(|p| p.unsigned_area()).collect()
    }

    /// A named attribute column, if present.
    pub fn attr(&self, name: &str) -> Option<&[f64]> {
        self.attrs.get(name).map(Vec::as_slice)
    }

    /// Set (or replace) a named attribute column. The column must be aligned
    /// 1:1 with this polygon set's rows.
    pub fn set_attr(&mut self, name: &str, values: Vec<f64>) -> Result<()> {
        if values.len() != self.polys.len() {
            return Err(GeoError::Parse(format!(
                "polygon attribute '{name}' has {} rows, expected {}",
                values.len(),
                self.polys.len()
            )));
        }
        self.attrs.insert(name.to_string(), values);
        self.record_history(format!("polygons.set_attr(name={name})"));
        Ok(())
    }

    /// The names of all attribute columns, in insertion order.
    pub fn attr_names(&self) -> Vec<&str> {
        self.attrs.keys().map(String::as_str).collect()
    }

    /// Human-readable operation history for this polygon set.
    pub fn history(&self) -> &[String] {
        self.history.entries()
    }

    pub(crate) fn record_history(&mut self, entry: impl Into<String>) {
        self.history.push(entry.into());
    }

    /// The exterior ring vertices of each polygon, as `[x, y, z]` with `z = 0.0`
    /// (Z is not retained — these sets are areal). The ring is closed (first
    /// vertex repeated last), in insertion order. Lets a consumer read the
    /// outline geometry, not just `area`/`bbox`/`contains`.
    pub fn rings(&self) -> Vec<Vec<[f64; 3]>> {
        self.polys
            .iter()
            .map(|p| p.exterior().coords().map(|c| [c.x, c.y, 0.0]).collect())
            .collect()
    }

    /// Axis-aligned bounding box over all polygons. Empty set → a box of `NaN`s.
    pub fn bbox(&self) -> BBox {
        let mut b = BBox {
            xmin: f64::INFINITY,
            ymin: f64::INFINITY,
            xmax: f64::NEG_INFINITY,
            ymax: f64::NEG_INFINITY,
        };
        let mut any = false;
        for poly in &self.polys {
            if let Some(rect) = poly.bounding_rect() {
                any = true;
                b.xmin = b.xmin.min(rect.min().x);
                b.ymin = b.ymin.min(rect.min().y);
                b.xmax = b.xmax.max(rect.max().x);
                b.ymax = b.ymax.max(rect.max().y);
            }
        }
        if any {
            b
        } else {
            BBox {
                xmin: f64::NAN,
                ymin: f64::NAN,
                xmax: f64::NAN,
                ymax: f64::NAN,
            }
        }
    }

    /// A copy of `surface` with every node *outside all* polygons masked to
    /// `NaN` (nodes inside any polygon keep their value). Geometry is preserved.
    pub fn clip(&self, surface: &Surface) -> Surface {
        let geom = &surface.geom;
        let src = surface.values();
        let mut out = Array2::from_elem((geom.ncol, geom.nrow), f64::NAN);
        for j in 0..geom.nrow {
            for i in 0..geom.ncol {
                let (x, y) = geom.node_xy(i, j);
                if self.contains(x, y) {
                    out[[i, j]] = src[[i, j]];
                }
            }
        }
        let mut clipped = Surface::from_values_unchecked(geom.clone(), out);
        let mut history = surface.operation_history().clone();
        history.extend_prefixed("mask", &self.history);
        history.push("polygons.clip(surface)".to_string());
        clipped.set_history(history);
        clipped
    }
}

impl HasHistory for PolygonSet {
    fn operation_history(&self) -> &OperationHistory {
        &self.history
    }

    fn operation_history_mut(&mut self) -> &mut OperationHistory {
        &mut self.history
    }
}

/// Andrew's monotone-chain convex hull over XY points; returns the hull vertices
/// counter-clockwise (no repeated closing vertex). Degenerate (collinear) inputs
/// may return fewer than three points.
pub(crate) fn convex_hull(mut pts: Vec<[f64; 2]>) -> Vec<[f64; 2]> {
    pts.sort_by(|a, b| a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1])));
    pts.dedup();
    if pts.len() < 3 {
        return pts;
    }
    let cross = |o: [f64; 2], a: [f64; 2], b: [f64; 2]| {
        (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0])
    };
    let mut lower: Vec<[f64; 2]> = Vec::new();
    for &p in &pts {
        while lower.len() >= 2 && cross(lower[lower.len() - 2], lower[lower.len() - 1], p) <= 0.0 {
            lower.pop();
        }
        lower.push(p);
    }
    let mut upper: Vec<[f64; 2]> = Vec::new();
    for &p in pts.iter().rev() {
        while upper.len() >= 2 && cross(upper[upper.len() - 2], upper[upper.len() - 1], p) <= 0.0 {
            upper.pop();
        }
        upper.push(p);
    }
    lower.pop();
    upper.pop();
    lower.extend(upper);
    lower
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::foundation::GridGeometry;

    /// A unit square [0,1]×[0,1].
    fn unit_square() -> PolygonSet {
        PolygonSet::from_rings(vec![vec![
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
        ]])
    }

    #[test]
    fn area_of_unit_square_is_one() {
        approx::assert_relative_eq!(unit_square().area(), 1.0);
    }

    #[test]
    fn rings_returns_exterior_vertices() {
        let rings = unit_square().rings();
        assert_eq!(rings.len(), 1);
        // Closed ring (first vertex repeated); all z = 0.
        assert_eq!(rings[0].first(), rings[0].last());
        assert!(rings[0].iter().all(|c| c[2] == 0.0));
        // The four corners are present.
        for corner in [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]] {
            assert!(rings[0]
                .iter()
                .any(|c| c[0] == corner[0] && c[1] == corner[1]));
        }
    }

    #[test]
    fn contains_interior_excludes_boundary() {
        let s = unit_square();
        assert!(s.contains(0.5, 0.5)); // interior
        assert!(!s.contains(2.0, 2.0)); // outside
                                        // geo excludes the boundary: a point exactly on an edge/corner is not
                                        // contained.
        assert!(!s.contains(0.0, 0.0)); // corner
        assert!(!s.contains(0.5, 0.0)); // edge midpoint
        assert!(!s.contains(1.0, 0.5)); // edge
    }

    #[test]
    fn bbox_covers_square() {
        let b = unit_square().bbox();
        approx::assert_relative_eq!(b.xmin, 0.0);
        approx::assert_relative_eq!(b.ymin, 0.0);
        approx::assert_relative_eq!(b.xmax, 1.0);
        approx::assert_relative_eq!(b.ymax, 1.0);
    }

    #[test]
    fn empty_set_is_nan_and_zero() {
        let s = PolygonSet::from_rings(Vec::new());
        assert!(s.bbox().xmin.is_nan());
        approx::assert_relative_eq!(s.area(), 0.0);
        assert!(!s.contains(0.0, 0.0));
    }

    #[test]
    fn clip_masks_nodes_outside_polygon() {
        // 3×3 grid on [0,2]×[0,2]; clip to a square covering only the lower-left
        // node neighbourhood [-0.5,1.5]² → nodes at x or y == 2 fall outside.
        let geom = GridGeometry {
            xori: 0.0,
            yori: 0.0,
            xinc: 1.0,
            yinc: 1.0,
            ncol: 3,
            nrow: 3,
            rotation_deg: 0.0,
            yflip: false,
        };
        let surf = Surface::constant(geom, 5.0);
        let poly = PolygonSet::from_rings(vec![vec![
            [-0.5, -0.5, 0.0],
            [1.5, -0.5, 0.0],
            [1.5, 1.5, 0.0],
            [-0.5, 1.5, 0.0],
        ]]);
        let clipped = poly.clip(&surf);
        let v = clipped.values();
        // inside: nodes (0,0),(1,0),(0,1),(1,1)
        assert_eq!(v[[0, 0]], 5.0);
        assert_eq!(v[[1, 1]], 5.0);
        // outside: anything with i==2 or j==2 (x==2 or y==2)
        assert!(v[[2, 0]].is_nan());
        assert!(v[[0, 2]].is_nan());
        assert!(v[[2, 2]].is_nan());
    }
}
