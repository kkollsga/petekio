//! `MeshShell` — the level-3 (unstructured) geometry shell.
//!
//! Integer node ids with explicit **2-D** XY, CCW triangle topology, the
//! quad-dominant wireframe, the boundary edge, and per-node walk labels
//! (`(block, i, j)` from the topology walk, where known). Purely
//! topological/positional — never a function of z; every value is a property
//! lane a surface maps onto this shell. Immutable once built; share via `Arc`.

use super::corner::CornerTable;
use super::fit::{fit_grid_from_coords, fit_grid_from_indexed};
use crate::core::PolygonSet;
use crate::foundation::{BBox, GeoError, GridGeometry, Result};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::OnceLock;

/// A node's place in the walked grid: fault block, and `(column, row)` inside it.
pub type WalkLabel = (u32, i32, i32);

/// An unstructured triangle-mesh shell (level 3). See the module docs.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(try_from = "MeshShellData")]
pub struct MeshShell {
    nodes: Vec<[f64; 2]>,
    triangles: Vec<[u32; 3]>,
    /// Unique triangle edges minus interior cell diagonals. Empty on legacy
    /// payloads (then [`wireframe_edges`](Self::wireframe_edges) falls back).
    wireframe: Vec<[u32; 2]>,
    edge: PolygonSet,
    /// Per-node walk label, `None` where the walk left the node unlabelled.
    labels: Vec<Option<WalkLabel>>,
    /// Derived walkability index — lazily built, never serialized.
    #[serde(skip)]
    corners: OnceLock<CornerTable>,
}

/// The serialized shape of a [`MeshShell`] (everything but the derived corner
/// table). Deserialization routes through [`MeshShell::new`] so a decoded
/// shell is always validated.
#[derive(serde::Serialize, serde::Deserialize)]
struct MeshShellData {
    nodes: Vec<[f64; 2]>,
    triangles: Vec<[u32; 3]>,
    #[serde(default)]
    wireframe: Vec<[u32; 2]>,
    edge: PolygonSet,
    #[serde(default)]
    labels: Vec<Option<WalkLabel>>,
}

impl TryFrom<MeshShellData> for MeshShell {
    type Error = GeoError;
    fn try_from(d: MeshShellData) -> Result<MeshShell> {
        let labels = if d.labels.is_empty() {
            vec![None; d.nodes.len()]
        } else {
            d.labels
        };
        MeshShell::new(d.nodes, d.triangles, d.wireframe, d.edge, labels)
    }
}

impl MeshShell {
    /// Build a shell from explicit parts. Validates that triangle indices are
    /// in range, that `labels` covers every node, and that **no undirected
    /// edge is carried by more than two triangles** (the shell must be
    /// edge-manifold — anything else is not a surface).
    pub fn new(
        nodes: Vec<[f64; 2]>,
        triangles: Vec<[u32; 3]>,
        wireframe: Vec<[u32; 2]>,
        edge: PolygonSet,
        labels: Vec<Option<WalkLabel>>,
    ) -> Result<MeshShell> {
        if labels.len() != nodes.len() {
            return Err(GeoError::GeometryMismatch(format!(
                "MeshShell::new: {} labels for {} nodes",
                labels.len(),
                nodes.len()
            )));
        }
        for e in &wireframe {
            if e[0] as usize >= nodes.len() || e[1] as usize >= nodes.len() {
                return Err(GeoError::GeometryMismatch(format!(
                    "MeshShell::new: wireframe edge ({}, {}) outside {} nodes",
                    e[0],
                    e[1],
                    nodes.len()
                )));
            }
        }
        // Validates triangle node references and edge-manifoldness in one pass;
        // memoize the result so the lazy accessor never re-does the work.
        let table = CornerTable::build(nodes.len(), &triangles)
            .map_err(|m| GeoError::GeometryInference(format!("MeshShell::new: {m}")))?;
        let corners = OnceLock::new();
        let _ = corners.set(table);
        Ok(MeshShell {
            nodes,
            triangles,
            wireframe,
            edge,
            labels,
            corners,
        })
    }

    /// Build a shell from nodes + CCW triangles + labels, deriving the
    /// quad-dominant wireframe and the boundary edge.
    pub(crate) fn from_triangles(
        nodes: Vec<[f64; 2]>,
        triangles: Vec<[u32; 3]>,
        labels: Vec<Option<WalkLabel>>,
    ) -> Result<MeshShell> {
        if labels.len() != nodes.len() {
            return Err(GeoError::GeometryMismatch(format!(
                "MeshShell::from_triangles: {} labels for {} nodes",
                labels.len(),
                nodes.len()
            )));
        }
        let wireframe = quad_wireframe(&triangles, &labels);
        let edge = boundary_rings(&triangles, &nodes)?;
        MeshShell::new(nodes, triangles, wireframe, edge, labels)
    }

    /// The node coordinates — 2-D by design; a shell is never a function of z.
    pub fn nodes(&self) -> &[[f64; 2]] {
        &self.nodes
    }

    /// Triangles as indices into [`nodes`](Self::nodes), counter-clockwise.
    pub fn triangles(&self) -> &[[u32; 3]] {
        &self.triangles
    }

    /// Per-node walk labels (`(block, i, j)`), `None` where unlabelled.
    pub fn labels(&self) -> &[Option<WalkLabel>] {
        &self.labels
    }

    pub fn n_nodes(&self) -> usize {
        self.nodes.len()
    }

    pub fn n_triangles(&self) -> usize {
        self.triangles.len()
    }

    /// Outer boundary ring(s) of the triangles.
    pub fn edge(&self) -> &PolygonSet {
        &self.edge
    }

    /// Unique triangle edges with interior cell diagonals removed — the
    /// quad-dominant wireframe (a full lattice cell shows as a square, not two
    /// triangles). Purely topological. Legacy payloads stored before the
    /// classification fall back to every unique edge.
    pub fn wireframe_edges(&self) -> Vec<[u32; 2]> {
        if !self.wireframe.is_empty() {
            return self.wireframe.clone();
        }
        let mut seen: BTreeSet<(u32, u32)> = BTreeSet::new();
        for t in &self.triangles {
            for &(a, b) in &[(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                seen.insert(if a <= b { (a, b) } else { (b, a) });
            }
        }
        seen.into_iter().map(|(a, b)| [a, b]).collect()
    }

    /// Axis-aligned bounding box over the nodes.
    pub fn bbox(&self) -> BBox {
        let mut b = BBox {
            xmin: f64::INFINITY,
            ymin: f64::INFINITY,
            xmax: f64::NEG_INFINITY,
            ymax: f64::NEG_INFINITY,
        };
        for p in &self.nodes {
            b.xmin = b.xmin.min(p[0]);
            b.xmax = b.xmax.max(p[0]);
            b.ymin = b.ymin.min(p[1]);
            b.ymax = b.ymax.max(p[1]);
        }
        b
    }

    /// Connected components (vertex connectivity). More than one means the
    /// shell honours a fault rather than bridging it.
    pub fn components(&self) -> usize {
        let tris: Vec<[usize; 3]> = self
            .triangles
            .iter()
            .map(|t| [t[0] as usize, t[1] as usize, t[2] as usize])
            .collect();
        count_components(&tris)
    }

    /// The derived corner table (opposite corners + vertex→corner). Lazily
    /// built on first use; never persisted.
    pub fn corner_table(&self) -> &CornerTable {
        self.corners.get_or_init(|| {
            CornerTable::build(self.nodes.len(), &self.triangles)
                .expect("shell validated at construction")
        })
    }

    /// Fit a regular [`GridGeometry`] to the shell's nodes (the lossy downward
    /// conversion). When the walk labelled every node into a single block the
    /// `(i, j)` indices drive an exact fit; otherwise the lattice is detected
    /// from bare coordinates. Errors when the shell is not regular within
    /// `tolerance`.
    pub fn infer_grid(&self, tolerance: f64) -> Result<GridGeometry> {
        let mut indexed = Vec::with_capacity(self.nodes.len());
        let mut single_block = true;
        let mut block = None;
        for (n, lab) in self.nodes.iter().zip(&self.labels) {
            match lab {
                Some((b, i, j)) if *block.get_or_insert(*b) == *b => {
                    indexed.push((*i as isize, *j as isize, n[0], n[1]));
                }
                _ => {
                    single_block = false;
                    break;
                }
            }
        }
        if single_block && indexed.len() == self.nodes.len() && !indexed.is_empty() {
            return fit_grid_from_indexed(&indexed, tolerance);
        }
        let coords: Vec<[f64; 3]> = self.nodes.iter().map(|n| [n[0], n[1], 0.0]).collect();
        fit_grid_from_coords(&coords, tolerance).map(|(g, _)| g)
    }
}

impl std::fmt::Debug for MeshShell {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MeshShell")
            .field("nodes", &self.nodes.len())
            .field("triangles", &self.triangles.len())
            .finish()
    }
}

impl GridGeometry {
    /// Explode the rigid grid into a [`MeshShell`]: every lattice node, each
    /// cell quad-split along a consistent diagonal (CCW triangles), lattice
    /// wireframe, perimeter edge, labels `(0, i, j)`. Lossless — node identity
    /// is `(i, j)`, carried on the labels. Errors on a degenerate (< 2×2) grid.
    pub fn to_mesh_shell(&self) -> Result<MeshShell> {
        self.to_structured_shell().to_mesh_shell()
    }
}

/// Unique triangle edges minus interior cell diagonals — the quad-dominant
/// wireframe of the **geometry**, which is a flat empty shell: purely
/// topological, never a function of z. A diagonal is hidden only when both
/// triangles of its cell survived (the edge is carried twice); a boundary
/// diagonal stays so its lone triangle keeps all three sides. Whether the
/// cell's four corners are coplanar is a property of the *shape* mapped onto
/// this shell and belongs to the surface layer, not here.
pub(crate) fn quad_wireframe(
    triangles: &[[u32; 3]],
    labels: &[Option<WalkLabel>],
) -> Vec<[u32; 2]> {
    let mut count: BTreeMap<(u32, u32), u8> = BTreeMap::new();
    for t in triangles {
        for &(a, b) in &[(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
            let key = if a <= b { (a, b) } else { (b, a) };
            *count.entry(key).or_insert(0) += 1;
        }
    }
    count
        .into_iter()
        .filter(|&((a, b), n)| {
            !(n == 2 && is_cell_diagonal(labels[a as usize], labels[b as usize]))
        })
        .map(|((a, b), _)| [a, b])
        .collect()
}

/// Are `a` and `b` opposite corners of one lattice cell in the same fault block?
fn is_cell_diagonal(a: Option<WalkLabel>, b: Option<WalkLabel>) -> bool {
    match (a, b) {
        (Some((ba, ia, ja)), Some((bb, ib, jb))) => {
            ba == bb && (ia - ib).abs() == 1 && (ja - jb).abs() == 1
        }
        _ => false,
    }
}

/// Chain the boundary edges into closed rings — the shell's outer boundary and
/// the outline of any interior hole.
///
/// The edges are **directed**, taken from each triangle's counter-clockwise
/// winding: a boundary edge is one whose reverse no triangle carries. Direction
/// matters. An undirected trace has to guess which way to leave a pinch vertex
/// — a vertex where the boundary touches itself — and the guess depends on hash
/// iteration order, so the same triangles yield different rings from run to
/// run. Following the winding leaves exactly one outgoing edge to take.
pub(crate) fn boundary_rings(tris: &[[u32; 3]], nodes: &[[f64; 2]]) -> Result<PolygonSet> {
    let directed: HashSet<(u32, u32)> = tris
        .iter()
        .flat_map(|t| [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])])
        .collect();
    let mut out: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    let mut remaining = 0usize;
    for &(a, b) in &directed {
        if !directed.contains(&(b, a)) {
            out.entry(a).or_default().push(b);
            remaining += 1;
        }
    }
    if remaining == 0 {
        return Err(GeoError::GeometryInference(
            "triangulated surface has no boundary".into(),
        ));
    }
    // Deterministic choice at a pinch vertex, where more than one boundary edge leaves.
    for vs in out.values_mut() {
        vs.sort_unstable();
    }
    let total = remaining;

    let mut rings = Vec::new();
    while let Some((&start, _)) = out.iter().find(|(_, vs)| !vs.is_empty()) {
        let mut ring = vec![start];
        let mut current = start;
        loop {
            let next = match out.get_mut(&current).and_then(|vs| vs.pop()) {
                Some(v) => v,
                None => {
                    return Err(GeoError::GeometryInference(
                        "triangulated surface boundary is not closed".into(),
                    ))
                }
            };
            if next == start {
                break;
            }
            ring.push(next);
            current = next;
            if ring.len() > total + 1 {
                return Err(GeoError::GeometryInference(
                    "triangulated surface boundary tracing did not close".into(),
                ));
            }
        }
        if ring.len() >= 3 {
            ring.push(start); // close it
            rings.push(
                ring.into_iter()
                    .map(|v| [nodes[v as usize][0], nodes[v as usize][1], 0.0])
                    .collect(),
            );
        }
    }
    if rings.is_empty() {
        return Err(GeoError::GeometryInference(
            "triangulated surface produced no closed boundary".into(),
        ));
    }
    Ok(PolygonSet::from_rings(rings))
}

/// Number of connected components among `tris` (vertex connectivity).
pub(crate) fn count_components(tris: &[[usize; 3]]) -> usize {
    let mut parent: HashMap<usize, usize> = HashMap::new();
    fn find(parent: &mut HashMap<usize, usize>, x: usize) -> usize {
        let mut root = x;
        while let Some(&p) = parent.get(&root) {
            if p == root {
                break;
            }
            root = p;
        }
        root
    }
    for t in tris {
        for &v in t {
            parent.entry(v).or_insert(v);
        }
    }
    for t in tris {
        let r0 = find(&mut parent, t[0]);
        for &v in &t[1..] {
            let r = find(&mut parent, v);
            if r != r0 {
                parent.insert(r, r0);
            }
        }
    }
    let roots: HashSet<usize> = tris.iter().map(|t| find(&mut parent, t[0])).collect();
    roots.len()
}

/// `2 ×` the signed area of triangle `(a, b, c)`; positive = counter-clockwise.
pub(crate) fn signed_area2(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> f64 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::shell::corner::NO_CORNER;
    use crate::foundation::GridGeometry;

    fn grid(rotation_deg: f64, yflip: bool) -> GridGeometry {
        GridGeometry {
            xori: 1000.0,
            yori: 2000.0,
            xinc: 50.0,
            yinc: 25.0,
            ncol: 5,
            nrow: 4,
            rotation_deg,
            yflip,
        }
    }

    #[test]
    fn grid_explodes_into_ccw_quad_split_mesh() {
        for (rot, yflip) in [(0.0, false), (30.0, false), (17.0, true)] {
            let g = grid(rot, yflip);
            let shell = g.to_mesh_shell().unwrap();
            assert_eq!(shell.n_nodes(), 5 * 4);
            assert_eq!(shell.n_triangles(), 2 * 4 * 3);
            // Every triangle is CCW in world coordinates.
            for t in shell.triangles() {
                let (a, b, c) = (
                    shell.nodes()[t[0] as usize],
                    shell.nodes()[t[1] as usize],
                    shell.nodes()[t[2] as usize],
                );
                assert!(
                    signed_area2(a, b, c) > 0.0,
                    "CW triangle at rot={rot} yflip={yflip}"
                );
            }
            // Wireframe = lattice edges only: 5*3 verticals + 4*4 horizontals.
            assert_eq!(shell.wireframe_edges().len(), 5 * 3 + 4 * 4);
            // Labels carry (0, i, j) for every node.
            assert!(shell.labels().iter().all(|l| matches!(l, Some((0, _, _)))));
            assert_eq!(shell.edge().rings().len(), 1);
        }
    }

    #[test]
    fn mesh_shell_infer_grid_round_trips_the_grid() {
        for (rot, yflip) in [(0.0, false), (30.0, false)] {
            let g = grid(rot, yflip);
            let shell = g.to_mesh_shell().unwrap();
            let back = shell.infer_grid(1e-6).unwrap();
            approx::assert_relative_eq!(back.xori, g.xori, epsilon = 1e-6);
            approx::assert_relative_eq!(back.yori, g.yori, epsilon = 1e-6);
            approx::assert_relative_eq!(back.xinc, g.xinc, epsilon = 1e-9);
            approx::assert_relative_eq!(back.yinc, g.yinc, epsilon = 1e-9);
            assert_eq!((back.ncol, back.nrow), (g.ncol, g.nrow));
        }
    }

    #[test]
    fn infer_grid_refuses_an_irregular_mesh() {
        let g = grid(0.0, false);
        let mut shell = g.to_mesh_shell().unwrap();
        // Warp half the nodes well off any lattice.
        let n = shell.nodes.len();
        for (k, p) in shell.nodes.iter_mut().enumerate() {
            if k % 2 == 0 {
                p[0] += 7.3 + (k % 5) as f64;
                p[1] -= 4.1 + (k % 3) as f64;
            }
        }
        assert_eq!(shell.labels.len(), n);
        assert!(shell.infer_grid(1e-3).is_err());
    }

    #[test]
    fn new_rejects_non_manifold_and_bad_indices() {
        let nodes = vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0], [2.0, 0.5]];
        let edge = PolygonSet::from_rings(vec![vec![
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0],
        ]]);
        // Edge (0,1) carried by three triangles: not a surface.
        let bad = vec![[0, 1, 2], [1, 0, 3], [0, 1, 4]];
        assert!(
            MeshShell::new(nodes.clone(), bad, Vec::new(), edge.clone(), vec![None; 5]).is_err()
        );
        // Out-of-range node id.
        let oob = vec![[0, 1, 9]];
        assert!(
            MeshShell::new(nodes.clone(), oob, Vec::new(), edge.clone(), vec![None; 5]).is_err()
        );
        // Label count mismatch.
        let ok = vec![[0, 1, 2]];
        assert!(MeshShell::new(nodes, ok, Vec::new(), edge, vec![None; 2]).is_err());
    }

    #[test]
    fn serde_round_trips_and_validates() {
        let shell = grid(20.0, false).to_mesh_shell().unwrap();
        // The persistence codec (bincode) is bit-exact.
        let bytes = crate::io::serial::to_bytes(&shell).unwrap();
        let back: MeshShell = crate::io::serial::from_bytes(&bytes).unwrap();
        assert_eq!(back.nodes(), shell.nodes());
        assert_eq!(back.triangles(), shell.triangles());
        assert_eq!(back.wireframe_edges(), shell.wireframe_edges());
        assert_eq!(back.labels(), shell.labels());
        // The corner table is derived, never serialized — and rebuilt lazily.
        let json = serde_json::to_string(&shell).unwrap();
        assert!(!json.contains("corners"));
        assert_eq!(back.corner_table().n_corners(), 3 * back.n_triangles());

        // A legacy payload without `wireframe` still loads; wireframe_edges
        // falls back to every unique edge.
        let mut v: serde_json::Value = serde_json::from_str(&json).unwrap();
        v.as_object_mut().unwrap().remove("wireframe");
        let legacy: MeshShell = serde_json::from_value(v).unwrap();
        assert!(legacy.wireframe_edges().len() > shell.wireframe_edges().len());

        // A corrupted payload (non-manifold) is refused at decode.
        let mut v: serde_json::Value = serde_json::from_str(&json).unwrap();
        let tris = v.get_mut("triangles").unwrap().as_array_mut().unwrap();
        let first = tris[0].clone();
        tris.push(first.clone());
        tris.push(first);
        assert!(serde_json::from_value::<MeshShell>(v).is_err());
    }

    #[test]
    fn corner_table_walks_across_shared_edges() {
        let shell = grid(0.0, false).to_mesh_shell().unwrap();
        let ct = shell.corner_table();
        // Every interior edge pairs two corners; boundary corners use the sentinel.
        let mut boundary = 0usize;
        for c in 0..ct.n_corners() as u32 {
            let o = ct.opposite(c);
            if o == NO_CORNER {
                boundary += 1;
            } else {
                assert_eq!(ct.opposite(o), c, "opposite must be symmetric");
            }
        }
        // The boundary corner count equals the number of boundary edges:
        // perimeter (2*(4+3) cells... on the split lattice: 2*ncells_x + 2*ncells_y).
        assert_eq!(boundary, 2 * 4 + 2 * 3);
    }
}
