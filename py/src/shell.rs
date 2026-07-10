//! Geometry-shell bindings — the flat empty shells behind level-2/3 surfaces
//! (`StructuredShell`, `MeshShell`), plus the shared iso-line / value-layer
//! marshalling helpers.

use crate::geometry::{BBox, GridGeometry};
use crate::points::PolygonSet;
use crate::to_pyerr;
use petekio::{
    MeshShell as RsMeshShell, StructuredShell as RsStructuredShell, ValueLayer as RsValueLayer,
};
use pyo3::prelude::*;
use pyo3::types::PyDict;
use std::sync::Arc;

/// The level-2 geometry shell: `(i, j)`-organized nodes with explicit per-node
/// XY. Purely topological/positional — never a function of z.
#[pyclass(name = "StructuredShell")]
pub struct StructuredShell {
    pub(crate) inner: Arc<RsStructuredShell>,
}

#[pymethods]
impl StructuredShell {
    #[getter]
    fn ncol(&self) -> usize {
        self.inner.ncol()
    }

    #[getter]
    fn nrow(&self) -> usize {
        self.inner.nrow()
    }

    /// X node coordinates as row-major nested lists: outer list is rows.
    fn x(&self) -> Vec<Vec<f64>> {
        matrix_rows(self.inner.x())
    }

    /// Y node coordinates as row-major nested lists: outer list is rows.
    fn y(&self) -> Vec<Vec<f64>> {
        matrix_rows(self.inner.y())
    }

    /// World `(x, y)` of logical node `(i, j)`.
    fn node_xy(&self, i: usize, j: usize) -> PyResult<(f64, f64)> {
        self.inner.node_xy(i, j).map_err(to_pyerr)
    }

    /// Optional approximate regular geometry (metadata only).
    #[getter]
    fn nominal_geometry(&self) -> Option<GridGeometry> {
        self.inner
            .nominal_geometry()
            .map(|g| GridGeometry::with_edge(g.clone(), self.inner.edge().clone()))
    }

    /// Edge polygon in modelling coordinates.
    #[getter]
    fn edge(&self) -> PolygonSet {
        PolygonSet::owned(self.inner.edge().clone())
    }

    /// Axis-aligned bounding box over finite XY nodes.
    fn bbox(&self) -> BBox {
        BBox::new(self.inner.bbox())
    }

    /// Explode into a level-3 `MeshShell` (free, lossless).
    fn to_mesh_shell(&self) -> PyResult<MeshShell> {
        self.inner
            .to_mesh_shell()
            .map(|m| MeshShell { inner: Arc::new(m) })
            .map_err(to_pyerr)
    }

    /// Fit a regular `GridGeometry` (lossy downward conversion); raises when
    /// the shell is curvilinear.
    #[pyo3(signature = (tolerance = 1e-3))]
    fn infer_grid(&self, tolerance: f64) -> PyResult<GridGeometry> {
        self.inner
            .infer_grid(tolerance)
            .map(|g| GridGeometry::with_edge(g, self.inner.edge().clone()))
            .map_err(to_pyerr)
    }

    fn __repr__(&self) -> String {
        format!(
            "StructuredShell(ncol={}, nrow={})",
            self.inner.ncol(),
            self.inner.nrow()
        )
    }
}

/// The level-3 geometry shell: integer node ids with explicit XY, CCW triangle
/// topology, the quad-dominant wireframe, the boundary edge, and per-node walk
/// labels. Purely topological/positional — never a function of z.
#[pyclass(name = "MeshShell")]
pub struct MeshShell {
    pub(crate) inner: Arc<RsMeshShell>,
}

#[pymethods]
impl MeshShell {
    #[getter]
    fn n_nodes(&self) -> usize {
        self.inner.n_nodes()
    }

    #[getter]
    fn n_triangles(&self) -> usize {
        self.inner.n_triangles()
    }

    /// Node XY as `(x, y)` tuples — 2-D by design (a shell has no z).
    fn nodes(&self) -> Vec<(f64, f64)> {
        self.inner.nodes().iter().map(|n| (n[0], n[1])).collect()
    }

    /// Triangles as `(i, j, k)` index triples into `nodes()`, CCW.
    fn triangles(&self) -> Vec<(u32, u32, u32)> {
        self.inner
            .triangles()
            .iter()
            .map(|t| (t[0], t[1], t[2]))
            .collect()
    }

    /// The quad-dominant wireframe as `(i, j)` index pairs into `nodes()`.
    /// `stride=k` (k ≥ 2) returns the coarse-LOD lattice wireframe (every k-th
    /// grid line per block, outline + seams + fringe kept); `None`/`1` is the
    /// full wireframe. Display-only — geometry is never decimated.
    #[pyo3(signature = (stride = None))]
    fn wireframe_edges(&self, stride: Option<usize>) -> Vec<(u32, u32)> {
        self.inner
            .wireframe_edges(stride)
            .into_iter()
            .map(|e| (e[0], e[1]))
            .collect()
    }

    /// Per-node walk labels `(block, i, j)`, `None` where unlabelled.
    fn labels(&self) -> Vec<Option<(u32, i32, i32)>> {
        self.inner.labels().to_vec()
    }

    /// Outer boundary ring(s) of the triangles.
    #[getter]
    fn edge(&self) -> PolygonSet {
        PolygonSet::owned(self.inner.edge().clone())
    }

    /// Connected components; more than one means the shell honours a fault.
    #[getter]
    fn components(&self) -> usize {
        self.inner.components()
    }

    /// Axis-aligned bounding box over the nodes.
    fn bbox(&self) -> BBox {
        BBox::new(self.inner.bbox())
    }

    /// Fit a regular `GridGeometry` (lossy downward conversion); raises when
    /// the shell is not regular.
    #[pyo3(signature = (tolerance = 1e-3))]
    fn infer_grid(&self, tolerance: f64) -> PyResult<GridGeometry> {
        self.inner
            .infer_grid(tolerance)
            .map(|g| GridGeometry::with_edge(g, self.inner.edge().clone()))
            .map_err(to_pyerr)
    }

    fn __repr__(&self) -> String {
        format!(
            "MeshShell(nodes={}, triangles={}, components={})",
            self.inner.n_nodes(),
            self.inner.n_triangles(),
            self.inner.components()
        )
    }
}

pub(crate) fn matrix_rows(a: &ndarray::Array2<f64>) -> Vec<Vec<f64>> {
    let (ncol, nrow) = a.dim();
    (0..nrow)
        .map(|j| (0..ncol).map(|i| a[[i, j]]).collect())
        .collect()
}

/// The marshalled iso-line payload: `list[tuple[level, list[list[(x, y)]]]]`.
pub(crate) type PyIsoLines = Vec<(f64, Vec<Vec<(f64, f64)>>)>;

/// Marshal iso-lines into [`PyIsoLines`].
pub(crate) fn iso_lines_py(out: Vec<(f64, Vec<Vec<[f64; 2]>>)>) -> PyIsoLines {
    out.into_iter()
        .map(|(level, lines)| {
            (
                level,
                lines
                    .into_iter()
                    .map(|line| line.into_iter().map(|p| (p[0], p[1])).collect())
                    .collect(),
            )
        })
        .collect()
}

/// Marshal a `ValueLayer` into the viewer's trimesh dict — the shape the
/// petektools viewer consumes; do not change it.
pub(crate) fn value_layer_dict(py: Python<'_>, layer: RsValueLayer) -> PyResult<Py<PyDict>> {
    let d = PyDict::new(py);
    d.set_item("kind", RsValueLayer::KIND)?;
    d.set_item("name", layer.name)?;
    d.set_item(
        "nodes",
        layer
            .nodes
            .iter()
            .map(|n| vec![n[0], n[1]])
            .collect::<Vec<_>>(),
    )?;
    d.set_item(
        "triangles",
        layer
            .triangles
            .iter()
            .map(|t| vec![t[0], t[1], t[2]])
            .collect::<Vec<_>>(),
    )?;
    d.set_item("values", layer.values)?;
    d.set_item("range", vec![layer.range[0], layer.range[1]])?;
    Ok(d.unbind())
}
