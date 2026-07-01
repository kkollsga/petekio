//! `PointSet` — scattered 3-D points (N×3 coords) with named `f64` attribute
//! columns, a spatial index for nearest-neighbour queries, and gridding onto a
//! `Surface` (`to_surface`). `NaN` = undefined. Imports from `foundation`,
//! `io`, and (for gridding) `core::surface`.

use crate::core::gridding;
use crate::core::surface::Surface;
use crate::foundation::{BBox, GridGeometry, Point3, Result, Stats};
use indexmap::IndexMap;
use rstar::primitives::GeomWithData;
use rstar::RTree;
use std::path::Path;

/// A gridding method for [`PointSet::to_surface`] — see
/// `dev-docs/designs/gridding-method.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GridMethod {
    /// Value of the single areally-closest sample (blocky, exact at data).
    Nearest,
    /// Inverse-distance weighting, `wᵢ = 1/dᵢ²` (power p=2), exact at d=0.
    InverseDistance,
    /// Briggs minimum-curvature (biharmonic SOR relaxation, data-anchored).
    MinimumCurvature,
}

/// An areal R*-tree entry: a 2-D `[x, y]` position carrying the point's index.
pub(crate) type AerialEntry = GeomWithData<[f64; 2], usize>;

/// Scattered points with attribute columns. Coordinates are stored as `[x, y,
/// z]`; each attribute is a `f64` column aligned 1:1 with `coords`.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct PointSet {
    pub(crate) coords: Vec<[f64; 3]>,
    pub(crate) attrs: IndexMap<String, Vec<f64>>,
}

impl PointSet {
    /// Build a `PointSet` from raw coordinates and attribute columns. Each
    /// attribute column must match `coords.len()` (callers within the crate
    /// guarantee this).
    pub(crate) fn from_parts(coords: Vec<[f64; 3]>, attrs: IndexMap<String, Vec<f64>>) -> PointSet {
        PointSet { coords, attrs }
    }

    /// Read a headered CSV, taking X/Y/Z from the named columns. Every other
    /// column whose values all parse as `f64` becomes an attribute; columns
    /// with any non-numeric cell are skipped. Rows with a non-numeric X/Y/Z are
    /// an error (readers validate on load).
    pub fn load_csv(path: impl AsRef<Path>, x: &str, y: &str, z: &str) -> Result<PointSet> {
        let (coords, attrs) = crate::io::csv_points::load(path.as_ref(), x, y, z)?;
        Ok(PointSet::from_parts(coords, attrs))
    }

    /// Load point features from a GeoJSON file. Each feature's numeric
    /// `properties{}` become attribute columns (the union of all features'
    /// numeric property names, NaN-filling features that lack one); string and
    /// other non-numeric properties are ignored.
    pub fn load_geojson(path: impl AsRef<Path>) -> Result<PointSet> {
        let (coords, attrs) = crate::io::vector::load_point_set_geojson(path.as_ref())?;
        Ok(PointSet::from_parts(coords, attrs))
    }

    /// Load scattered points from an IRAP/RMS plain `X Y Z` file. No named
    /// attributes (the format carries none).
    pub fn load_irap_points(path: impl AsRef<Path>) -> Result<PointSet> {
        let coords = crate::io::xyz::load_points(path.as_ref())?;
        Ok(PointSet::from_parts(coords, IndexMap::new()))
    }

    /// Number of points.
    pub fn len(&self) -> usize {
        self.coords.len()
    }

    /// Whether the set is empty.
    pub fn is_empty(&self) -> bool {
        self.coords.is_empty()
    }

    /// A new `PointSet` keeping only points for which `pred` is true. Attribute
    /// columns are carried over for the retained rows.
    pub fn filter(&self, pred: impl Fn(Point3) -> bool) -> PointSet {
        let keep: Vec<usize> = (0..self.coords.len())
            .filter(|&i| {
                let c = self.coords[i];
                pred(Point3::new(c[0], c[1], c[2]))
            })
            .collect();
        let coords = keep.iter().map(|&i| self.coords[i]).collect();
        let attrs = self
            .attrs
            .iter()
            .map(|(name, col)| (name.clone(), keep.iter().map(|&i| col[i]).collect()))
            .collect();
        PointSet::from_parts(coords, attrs)
    }

    /// A named attribute column, if present.
    pub fn attr(&self, name: &str) -> Option<&[f64]> {
        self.attrs.get(name).map(Vec::as_slice)
    }

    /// NaN-skipping statistics over a named attribute column, or `None` if the
    /// attribute is absent.
    pub fn stats(&self, attr: &str) -> Option<Stats> {
        self.attrs.get(attr).map(|col| Stats::of(col))
    }

    /// Axis-aligned bounding box of the points' XY. Empty set → a degenerate
    /// box of `NaN`s.
    pub fn bbox(&self) -> BBox {
        let mut b = BBox {
            xmin: f64::INFINITY,
            ymin: f64::INFINITY,
            xmax: f64::NEG_INFINITY,
            ymax: f64::NEG_INFINITY,
        };
        for c in &self.coords {
            b.xmin = b.xmin.min(c[0]);
            b.xmax = b.xmax.max(c[0]);
            b.ymin = b.ymin.min(c[1]);
            b.ymax = b.ymax.max(c[1]);
        }
        if self.coords.is_empty() {
            b = BBox {
                xmin: f64::NAN,
                ymin: f64::NAN,
                xmax: f64::NAN,
                ymax: f64::NAN,
            };
        }
        b
    }

    /// Index of the areally-nearest point to `(x, y)` (Euclidean in XY; Z is
    /// ignored). `None` for an empty set.
    pub fn nearest(&self, x: f64, y: f64) -> Option<usize> {
        if self.coords.is_empty() {
            return None;
        }
        let tree = self.rtree_xy();
        tree.nearest_neighbor([x, y]).map(|e| e.data)
    }

    /// Grid the points' Z values onto `geom` using `method`, returning a new
    /// `Surface`. See `dev-docs/designs/gridding-method.md`.
    pub fn to_surface(&self, geom: GridGeometry, method: GridMethod) -> Result<Surface> {
        gridding::grid(&self.coords, geom, method)
    }

    /// Warm-started minimum-curvature re-grid onto `prior`'s lattice, relaxing
    /// from `prior`'s values instead of a cold IDW seed. For an incremental
    /// re-grid (control points nudged, a point added) this converges much faster
    /// than [`to_surface`](Self::to_surface) with `MinimumCurvature` while giving
    /// the same converged field. Honours the points as hard constraints, as the
    /// cold path does.
    pub fn regrid_min_curvature(&self, prior: &Surface) -> Result<Surface> {
        gridding::grid_min_curvature_warm(&self.coords, prior.geom.clone(), prior.values())
    }

    /// Build an areal R*-tree over the points' XY, payloaded with their index.
    pub(crate) fn rtree_xy(&self) -> RTree<AerialEntry> {
        gridding::build_rtree(&self.coords)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pts() -> PointSet {
        let coords = vec![[0.0, 0.0, 1.0], [10.0, 0.0, 2.0], [0.0, 10.0, 3.0]];
        let mut attrs = IndexMap::new();
        attrs.insert("poro".to_string(), vec![0.1, 0.2, 0.3]);
        PointSet::from_parts(coords, attrs)
    }

    #[test]
    fn len_and_attr_and_stats() {
        let p = pts();
        assert_eq!(p.len(), 3);
        assert!(!p.is_empty());
        assert_eq!(p.attr("poro").unwrap(), &[0.1, 0.2, 0.3]);
        assert!(p.attr("missing").is_none());
        let s = p.stats("poro").unwrap();
        assert_eq!(s.count, 3);
        approx::assert_relative_eq!(s.mean, 0.2);
        assert!(p.stats("nope").is_none());
    }

    #[test]
    fn bbox_covers_points() {
        let b = pts().bbox();
        approx::assert_relative_eq!(b.xmin, 0.0);
        approx::assert_relative_eq!(b.xmax, 10.0);
        approx::assert_relative_eq!(b.ymin, 0.0);
        approx::assert_relative_eq!(b.ymax, 10.0);
    }

    #[test]
    fn nearest_matches_brute_force() {
        let p = pts();
        // brute-force nearest to a few query points
        let queries = [(1.0, 1.0), (9.0, 1.0), (1.0, 9.0), (5.0, 5.0)];
        for (qx, qy) in queries {
            let brute = (0..p.len())
                .min_by(|&a, &b| {
                    let da = (p.coords[a][0] - qx).powi(2) + (p.coords[a][1] - qy).powi(2);
                    let db = (p.coords[b][0] - qx).powi(2) + (p.coords[b][1] - qy).powi(2);
                    da.total_cmp(&db)
                })
                .unwrap();
            assert_eq!(p.nearest(qx, qy), Some(brute));
        }
    }

    #[test]
    fn filter_keeps_matching_rows_and_attrs() {
        let p = pts().filter(|pt| pt.x < 5.0);
        assert_eq!(p.len(), 2); // (0,0) and (0,10)
        assert_eq!(p.attr("poro").unwrap(), &[0.1, 0.3]);
    }

    #[test]
    fn empty_nearest_is_none() {
        let p = PointSet::from_parts(Vec::new(), IndexMap::new());
        assert!(p.is_empty());
        assert!(p.nearest(0.0, 0.0).is_none());
    }

    fn grid5() -> crate::foundation::GridGeometry {
        crate::foundation::GridGeometry {
            xori: 0.0,
            yori: 0.0,
            xinc: 2.5,
            yinc: 2.5,
            ncol: 5,
            nrow: 5,
            rotation_deg: 0.0,
            yflip: false,
        }
    }

    #[test]
    fn warm_start_honours_constraints_and_converges() {
        let p = pts();
        let cold = p.to_surface(grid5(), GridMethod::MinimumCurvature).unwrap();
        let warm = p.regrid_min_curvature(&cold).unwrap();
        assert_eq!(warm.geom, cold.geom);
        // Hard constraints: each input point snaps to a node held at its z.
        // Points (0,0,1)→node[0,0], (10,0,2)→node[4,0], (0,10,3)→node[0,4].
        approx::assert_relative_eq!(warm.values()[[0, 0]], 1.0, epsilon = 1e-9);
        approx::assert_relative_eq!(warm.values()[[4, 0]], 2.0, epsilon = 1e-9);
        approx::assert_relative_eq!(warm.values()[[0, 4]], 3.0, epsilon = 1e-9);
        // A second warm pass is a near-fixed point (the field has converged).
        let warm2 = p.regrid_min_curvature(&warm).unwrap();
        for (a, b) in warm2.values().iter().zip(warm.values().iter()) {
            approx::assert_relative_eq!(a, b, epsilon = 1e-6);
        }
    }

    #[test]
    fn regrid_empty_errors() {
        let empty = PointSet::from_parts(Vec::new(), IndexMap::new());
        let prior = pts()
            .to_surface(grid5(), GridMethod::MinimumCurvature)
            .unwrap();
        assert!(empty.regrid_min_curvature(&prior).is_err());
    }
}
