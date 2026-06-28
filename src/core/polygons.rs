//! `PolygonSet` — a collection of polygon rings (boundaries, faults, fluid
//! contacts) with point-in-polygon, area, bounding box, and surface clipping.
//! Backed by the `geo` crate's 2-D predicates. Imports from `foundation`, `io`,
//! and `core::surface`.

use crate::core::surface::Surface;
use crate::foundation::{BBox, Result};
use geo::prelude::*;
use geo::{Coord, LineString, Point, Polygon};
use ndarray::Array2;
use std::path::Path;

/// A set of simple polygons (exterior rings, no holes). Z, if present in the
/// source, is dropped — all operations are areal. Undefined extents on an empty
/// set are `NaN`.
pub struct PolygonSet {
    polys: Vec<Polygon<f64>>,
}

impl PolygonSet {
    /// Build from rings of `[x, y, z]` coordinates (Z ignored). Rings with
    /// fewer than three vertices are dropped (no area). `geo` closes each
    /// exterior ring automatically.
    pub(crate) fn from_rings(rings: Vec<Vec<[f64; 3]>>) -> PolygonSet {
        let polys = rings
            .into_iter()
            .filter(|r| r.len() >= 3)
            .map(|r| {
                let coords: Vec<Coord<f64>> =
                    r.iter().map(|c| Coord { x: c[0], y: c[1] }).collect();
                Polygon::new(LineString::new(coords), Vec::new())
            })
            .collect();
        PolygonSet { polys }
    }

    /// Load polygons from an IRAP/RMS plain `X Y Z` file (rings separated by the
    /// `999.0` sentinel).
    pub fn load_irap_polygons(path: impl AsRef<Path>) -> Result<PolygonSet> {
        let rings = crate::io::xyz::load_polygons(path.as_ref())?;
        Ok(PolygonSet::from_rings(rings))
    }

    /// Whether `(x, y)` is inside any polygon. Uses `geo`'s `Contains`, which
    /// **excludes** the boundary (a point exactly on an edge is *not*
    /// contained).
    pub fn contains(&self, x: f64, y: f64) -> bool {
        let p = Point::new(x, y);
        self.polys.iter().any(|poly| poly.contains(&p))
    }

    /// Total unsigned area of all polygons (summed; overlaps double-count).
    pub fn area(&self) -> f64 {
        self.polys.iter().map(|p| p.unsigned_area()).sum()
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
        Surface::from_values_unchecked(geom.clone(), out)
    }
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
