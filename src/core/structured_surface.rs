//! `StructuredMeshSurface` — a logically regular surface whose nodes carry
//! explicit XY coordinates.
//!
//! This is the Petrel/fault-shifted surface home: the surface has a rectangular
//! `(column, row)` topology, but it does **not** claim that all nodes lie on one
//! global affine `GridGeometry`. Regular gridded surfaces stay in [`Surface`].

use crate::core::{PointSet, PolygonSet};
use crate::foundation::{
    BBox, GeoError, GridGeometry, HasHistory, OperationHistory, Result, Stats,
};
use indexmap::IndexMap;
use ndarray::Array2;

/// A logically regular surface with explicit per-node coordinates.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct StructuredMeshSurface {
    ncol: usize,
    nrow: usize,
    x: Array2<f64>,
    y: Array2<f64>,
    values: Array2<f64>,
    nominal_geometry: Option<GridGeometry>,
    edge: PolygonSet,
    #[serde(default)]
    history: OperationHistory,
}

impl StructuredMeshSurface {
    /// Build a structured mesh surface from explicit node coordinate/value
    /// arrays. Arrays must all be shaped `(ncol, nrow)`.
    pub fn new(
        x: Array2<f64>,
        y: Array2<f64>,
        values: Array2<f64>,
        nominal_geometry: Option<GridGeometry>,
        edge: PolygonSet,
    ) -> Result<Self> {
        let shape = x.dim();
        if shape.0 == 0 || shape.1 == 0 {
            return Err(GeoError::GeometryMismatch(
                "StructuredMeshSurface::new: shape must be non-empty".into(),
            ));
        }
        if y.dim() != shape || values.dim() != shape {
            return Err(GeoError::GeometryMismatch(format!(
                "StructuredMeshSurface::new: x/y/values shapes differ: x={:?}, y={:?}, values={:?}",
                x.dim(),
                y.dim(),
                values.dim()
            )));
        }
        Ok(Self {
            ncol: shape.0,
            nrow: shape.1,
            x,
            y,
            values,
            nominal_geometry,
            edge,
            history: OperationHistory::from_entry("structured_surface.new"),
        })
    }

    /// Stable kind label for dispatch/reporting.
    pub fn kind(&self) -> &'static str {
        "structured_mesh"
    }

    pub fn ncol(&self) -> usize {
        self.ncol
    }

    pub fn nrow(&self) -> usize {
        self.nrow
    }

    pub fn x(&self) -> &Array2<f64> {
        &self.x
    }

    pub fn y(&self) -> &Array2<f64> {
        &self.y
    }

    pub fn values(&self) -> &Array2<f64> {
        &self.values
    }

    /// Optional approximate regular geometry. This is metadata only; consumers
    /// must not treat it as the canonical node coordinate model.
    pub fn nominal_geometry(&self) -> Option<&GridGeometry> {
        self.nominal_geometry.as_ref()
    }

    /// Edge polygon in modelling coordinates.
    pub fn edge(&self) -> &PolygonSet {
        &self.edge
    }

    /// World `(x, y)` of logical node `(i, j)`.
    pub fn node_xy(&self, i: usize, j: usize) -> Result<(f64, f64)> {
        self.check_node(i, j)?;
        Ok((self.x[[i, j]], self.y[[i, j]]))
    }

    /// Primary value at logical node `(i, j)`.
    pub fn z(&self, i: usize, j: usize) -> Result<f64> {
        self.check_node(i, j)?;
        Ok(self.values[[i, j]])
    }

    /// Explode the mesh back into a [`PointSet`] — one point per populated node,
    /// carrying its `column`/`row` topology.
    ///
    /// Exact by construction: node XY/Z are **copied**, never resampled, so this is
    /// the inverse of [`PointSet::to_structured_surface`](crate::PointSet::to_structured_surface).
    /// Node order is row-major (`column` varies fastest) and the emitted indices are
    /// renumbered 1-based, so a round trip preserves every coordinate and the
    /// topology, though not an arbitrary input ordering or index origin.
    pub fn to_points(&self) -> PointSet {
        let mut coords = Vec::new();
        let mut columns = Vec::new();
        let mut rows = Vec::new();
        for j in 0..self.nrow {
            for i in 0..self.ncol {
                let (x, y) = (self.x[[i, j]], self.y[[i, j]]);
                if !x.is_finite() || !y.is_finite() {
                    continue;
                }
                coords.push([x, y, self.values[[i, j]]]);
                columns.push((i + 1) as f64);
                rows.push((j + 1) as f64);
            }
        }
        let mut attrs = IndexMap::new();
        attrs.insert("column".to_string(), columns);
        attrs.insert("row".to_string(), rows);
        let mut out = PointSet::from_parts(coords, attrs);
        *out.operation_history_mut() = self.history.clone();
        out.record_history("structured_surface.to_points()");
        out
    }

    /// Summary statistics over finite primary values.
    pub fn stats(&self) -> Stats {
        Stats::of(&self.values.iter().copied().collect::<Vec<_>>())
    }

    /// Axis-aligned bounding box over finite XY nodes.
    pub fn bbox(&self) -> BBox {
        let mut b = BBox {
            xmin: f64::INFINITY,
            ymin: f64::INFINITY,
            xmax: f64::NEG_INFINITY,
            ymax: f64::NEG_INFINITY,
        };
        let mut any = false;
        for (x, y) in self.x.iter().zip(self.y.iter()) {
            if x.is_finite() && y.is_finite() {
                any = true;
                b.xmin = b.xmin.min(*x);
                b.xmax = b.xmax.max(*x);
                b.ymin = b.ymin.min(*y);
                b.ymax = b.ymax.max(*y);
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

    /// Human-readable operation history.
    pub fn history(&self) -> &[String] {
        self.history.entries()
    }

    pub(crate) fn set_history(&mut self, history: impl Into<OperationHistory>) {
        self.history = history.into();
    }

    fn check_node(&self, i: usize, j: usize) -> Result<()> {
        if i >= self.ncol || j >= self.nrow {
            return Err(GeoError::OutOfRange(format!(
                "structured surface node ({i}, {j}) outside shape (ncol={}, nrow={})",
                self.ncol, self.nrow
            )));
        }
        Ok(())
    }
}

impl HasHistory for StructuredMeshSurface {
    fn operation_history(&self) -> &OperationHistory {
        &self.history
    }

    fn operation_history_mut(&mut self) -> &mut OperationHistory {
        &mut self.history
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ndarray::array;

    #[test]
    fn keeps_explicit_shifted_node_coordinates() {
        let x = array![[0.0, 0.0], [10.0, 12.0]];
        let y = array![[0.0, 10.0], [0.0, 10.0]];
        let z = array![[100.0, 110.0], [120.0, 130.0]];
        let edge =
            PolygonSet::convex_hull_xy(vec![[0.0, 0.0], [10.0, 0.0], [12.0, 10.0], [0.0, 10.0]])
                .unwrap();

        let s = StructuredMeshSurface::new(x, y, z, None, edge).unwrap();
        assert_eq!(s.kind(), "structured_mesh");
        assert_eq!(s.ncol(), 2);
        assert_eq!(s.nrow(), 2);
        assert_eq!(s.node_xy(1, 1).unwrap(), (12.0, 10.0));
        assert_eq!(s.z(1, 1).unwrap(), 130.0);
        assert_eq!(s.stats().count, 4);
    }

    #[test]
    fn points_to_mesh_to_points_is_bit_for_bit_exact() {
        // A curvilinear, partially populated mesh with a locally shifted node — the
        // shape petekIO must carry without moving a single coordinate.
        let (ncol, nrow) = (7usize, 5usize);
        let mut coords = Vec::new();
        let mut columns = Vec::new();
        let mut rows = Vec::new();
        for j in 0..nrow {
            for i in 0..ncol {
                if i >= 5 && j >= 3 {
                    continue; // an unpopulated corner: the footprint is not rectangular
                }
                let swell = 1.0 + 0.07 * i as f64;
                let mut x = 1000.0 + 50.0 * i as f64 * swell;
                let mut y = 2000.0 + 50.0 * j as f64 * (1.0 + 0.05 * j as f64);
                if i == 2 && j == 2 {
                    x += 9.75; // a fault-shifted node
                    y -= 4.5;
                }
                coords.push([x, y, -1800.0 - (i * j) as f64]);
                columns.push((i + 1) as f64);
                rows.push((j + 1) as f64);
            }
        }
        let mut attrs = IndexMap::new();
        attrs.insert("column".to_string(), columns);
        attrs.insert("row".to_string(), rows);
        let original = PointSet::from_parts(coords, attrs);

        let mesh = original
            .to_structured_surface(1e-3, crate::GeometryEdge::Occupied)
            .expect("a curvilinear mesh is exactly representable");
        let back = mesh.to_points();

        assert_eq!(back.len(), original.len());
        // Key both sides by (column, row) and demand exact f64 equality — no epsilon.
        let key = |p: &PointSet| {
            let c = p.attr("column").unwrap().to_vec();
            let r = p.attr("row").unwrap().to_vec();
            let mut v: Vec<((u64, u64), [f64; 3])> = p
                .coords()
                .iter()
                .enumerate()
                .map(|(k, xyz)| ((c[k] as u64, r[k] as u64), *xyz))
                .collect();
            v.sort_by_key(|(k, _)| *k);
            v
        };
        let (a, b) = (key(&original), key(&back));
        assert_eq!(a.len(), b.len());
        for ((ka, xa), (kb, xb)) in a.iter().zip(b.iter()) {
            assert_eq!(ka, kb, "topology must survive the round trip");
            assert_eq!(
                xa, xb,
                "coordinates must be bit-for-bit identical at {ka:?}"
            );
        }
    }
}
