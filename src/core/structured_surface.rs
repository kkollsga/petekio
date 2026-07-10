//! `StructuredMeshSurface` — a level-2 surface: an [`StructuredShell`]
//! (geometry, `Arc`-shared) plus a primary value lane and named attribute
//! lanes, all shaped `(ncol, nrow)`.
//!
//! This is the Petrel/fault-shifted surface home: the surface has a rectangular
//! `(column, row)` topology, but it does **not** claim that all nodes lie on one
//! global affine `GridGeometry`. Regular gridded surfaces stay in [`Surface`];
//! fault-cut surfaces with no `(column, row)` space at all live in
//! [`TriSurface`](crate::TriSurface). The shell is immutable and shared — N
//! properties/clones never repeat the geometry in memory.

use crate::core::shell::StructuredShell;
use crate::core::{PointSet, PolygonSet};
use crate::foundation::{
    BBox, GeoError, GridGeometry, HasHistory, OperationHistory, Result, Stats,
};
use indexmap::IndexMap;
use ndarray::Array2;
use std::sync::Arc;

/// A logically regular surface with explicit per-node coordinates: shell +
/// primary values + attribute lanes.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(try_from = "StructuredMeshSurfaceData")]
pub struct StructuredMeshSurface {
    shell: Arc<StructuredShell>,
    values: Array2<f64>,
    attributes: IndexMap<String, Array2<f64>>,
    #[serde(default)]
    history: OperationHistory,
}

/// Serialized shape: the shell **once**, then N property lanes referencing it.
/// Deserialization routes through the validating constructor.
#[derive(serde::Serialize, serde::Deserialize)]
struct StructuredMeshSurfaceData {
    shell: StructuredShell,
    values: Array2<f64>,
    #[serde(default)]
    attributes: IndexMap<String, Array2<f64>>,
    #[serde(default)]
    history: OperationHistory,
}

impl TryFrom<StructuredMeshSurfaceData> for StructuredMeshSurface {
    type Error = GeoError;
    fn try_from(d: StructuredMeshSurfaceData) -> Result<StructuredMeshSurface> {
        let mut out = StructuredMeshSurface::from_shell(Arc::new(d.shell), d.values)?;
        for (name, lane) in d.attributes {
            out.set_attr(&name, lane)?;
        }
        out.history = d.history;
        Ok(out)
    }
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
        let shell = StructuredShell::new(x, y, nominal_geometry, edge)?;
        let mut out = Self::from_shell(Arc::new(shell), values)?;
        out.history = OperationHistory::from_entry("structured_surface.new");
        Ok(out)
    }

    /// Build a surface over an existing (shared) shell. The value lane must
    /// match the shell's `(ncol, nrow)` shape.
    pub fn from_shell(shell: Arc<StructuredShell>, values: Array2<f64>) -> Result<Self> {
        check_lane(&shell, &values, "StructuredMeshSurface::from_shell")?;
        Ok(Self {
            shell,
            values,
            attributes: IndexMap::new(),
            history: OperationHistory::from_entry("structured_surface.from_shell"),
        })
    }

    /// Stable kind label for dispatch/reporting.
    pub fn kind(&self) -> &'static str {
        "structured_mesh"
    }

    /// The geometry shell (shared; never copied per property lane).
    pub fn shell(&self) -> &Arc<StructuredShell> {
        &self.shell
    }

    pub fn ncol(&self) -> usize {
        self.shell.ncol()
    }

    pub fn nrow(&self) -> usize {
        self.shell.nrow()
    }

    pub fn x(&self) -> &Array2<f64> {
        self.shell.x()
    }

    pub fn y(&self) -> &Array2<f64> {
        self.shell.y()
    }

    /// The primary value lane, shape `(ncol, nrow)`. `NaN` = undefined.
    pub fn values(&self) -> &Array2<f64> {
        &self.values
    }

    /// A named attribute lane, if present.
    pub fn attr(&self, name: &str) -> Option<&Array2<f64>> {
        self.attributes.get(name)
    }

    /// Set (or replace) a named attribute lane. Must match the shell shape or
    /// `GeometryMismatch` is returned.
    pub fn set_attr(&mut self, name: &str, values: Array2<f64>) -> Result<()> {
        check_lane(&self.shell, &values, "StructuredMeshSurface::set_attr")?;
        self.attributes.insert(name.to_string(), values);
        self.record_history(format!("structured_surface.set_attr(name={name})"));
        Ok(())
    }

    /// The names of all attribute lanes, in insertion order.
    pub fn attr_names(&self) -> Vec<&str> {
        self.attributes.keys().map(String::as_str).collect()
    }

    /// Promote an attribute lane to a standalone surface (its primary values)
    /// on the **same shared shell** — no geometry is copied.
    pub fn as_attr_surface(&self, name: &str) -> Option<StructuredMeshSurface> {
        self.attributes.get(name).map(|a| StructuredMeshSurface {
            shell: Arc::clone(&self.shell),
            values: a.clone(),
            attributes: IndexMap::new(),
            history: self
                .history
                .with_entry(format!("structured_surface.as_attr_surface(name={name})")),
        })
    }

    /// Optional approximate regular geometry. This is metadata only; consumers
    /// must not treat it as the canonical node coordinate model.
    pub fn nominal_geometry(&self) -> Option<&GridGeometry> {
        self.shell.nominal_geometry()
    }

    /// Edge polygon in modelling coordinates.
    pub fn edge(&self) -> &PolygonSet {
        self.shell.edge()
    }

    /// World `(x, y)` of logical node `(i, j)`.
    pub fn node_xy(&self, i: usize, j: usize) -> Result<(f64, f64)> {
        self.shell.node_xy(i, j)
    }

    /// Primary value at logical node `(i, j)`.
    pub fn z(&self, i: usize, j: usize) -> Result<f64> {
        self.shell.check_node(i, j)?;
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
        let (ncol, nrow) = (self.ncol(), self.nrow());
        let (x, y) = (self.shell.x(), self.shell.y());
        let mut coords = Vec::new();
        let mut columns = Vec::new();
        let mut rows = Vec::new();
        for j in 0..nrow {
            for i in 0..ncol {
                let (px, py) = (x[[i, j]], y[[i, j]]);
                if !px.is_finite() || !py.is_finite() {
                    continue;
                }
                coords.push([px, py, self.values[[i, j]]]);
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

    /// Fit a regular [`GridGeometry`] to the shell (the lossy downward
    /// conversion); errors when the mesh is curvilinear. Delegates to
    /// [`StructuredShell::infer_grid`].
    pub fn infer_grid(&self, tolerance: f64) -> Result<GridGeometry> {
        self.shell.infer_grid(tolerance)
    }

    /// Summary statistics over finite primary values.
    pub fn stats(&self) -> Stats {
        Stats::of(&self.values.iter().copied().collect::<Vec<_>>())
    }

    /// Axis-aligned bounding box over finite XY nodes.
    pub fn bbox(&self) -> BBox {
        self.shell.bbox()
    }

    /// Human-readable operation history.
    pub fn history(&self) -> &[String] {
        self.history.entries()
    }

    pub(crate) fn set_history(&mut self, history: impl Into<OperationHistory>) {
        self.history = history.into();
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

fn check_lane(shell: &StructuredShell, lane: &Array2<f64>, ctx: &str) -> Result<()> {
    if lane.dim() != (shell.ncol(), shell.nrow()) {
        return Err(GeoError::GeometryMismatch(format!(
            "{ctx}: lane shape {:?} != shell (ncol={}, nrow={})",
            lane.dim(),
            shell.ncol(),
            shell.nrow()
        )));
    }
    Ok(())
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
    fn attribute_lanes_share_one_shell() {
        let x = array![[0.0, 0.0], [10.0, 10.0]];
        let y = array![[0.0, 10.0], [0.0, 10.0]];
        let z = array![[1.0, 2.0], [3.0, 4.0]];
        let edge =
            PolygonSet::convex_hull_xy(vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]])
                .unwrap();
        let mut s = StructuredMeshSurface::new(x, y, z, None, edge).unwrap();
        s.set_attr("thickness", array![[5.0, 5.0], [5.0, f64::NAN]])
            .unwrap();
        assert_eq!(s.attr_names(), vec!["thickness"]);
        assert!(s.attr("missing").is_none());
        // wrong-shape lane rejected
        assert!(s.set_attr("bad", Array2::zeros((3, 3))).is_err());

        let promoted = s.as_attr_surface("thickness").unwrap();
        assert_eq!(promoted.values()[[0, 0]], 5.0);
        assert_eq!(promoted.stats().count, 3); // NaN skipped
                                               // The shell is shared, not copied.
        assert!(Arc::ptr_eq(s.shell(), promoted.shell()));
    }

    #[test]
    fn serde_round_trips_shell_and_lanes() {
        let x = array![[0.0, 0.0], [10.0, 12.0]];
        let y = array![[0.0, 10.0], [0.0, 10.0]];
        let z = array![[100.0, f64::NAN], [120.0, 130.0]];
        let edge =
            PolygonSet::convex_hull_xy(vec![[0.0, 0.0], [10.0, 0.0], [12.0, 10.0], [0.0, 10.0]])
                .unwrap();
        let mut s = StructuredMeshSurface::new(x, y, z, None, edge).unwrap();
        s.set_attr("amp", array![[0.1, 0.2], [0.3, 0.4]]).unwrap();

        // The persistence codec (bincode) is NaN-safe and bit-exact.
        let bytes = crate::io::serial::to_bytes(&s).unwrap();
        let back: StructuredMeshSurface = crate::io::serial::from_bytes(&bytes).unwrap();
        assert_eq!(back.ncol(), 2);
        assert_eq!(back.node_xy(1, 1).unwrap(), (12.0, 10.0));
        assert!(back.z(0, 1).unwrap().is_nan());
        assert_eq!(back.attr("amp").unwrap()[[1, 1]], 0.4);
        // The shell appears exactly once in the payload (JSON projection).
        let json = serde_json::to_string(&back.as_attr_surface("amp").unwrap()).unwrap();
        assert_eq!(json.matches("\"shell\"").count(), 1);
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
