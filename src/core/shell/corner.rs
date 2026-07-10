//! The corner table — [`MeshShell`](super::MeshShell)'s derived walkability
//! index.
//!
//! A *corner* is one vertex slot of one triangle: triangle `t`'s corners are
//! `3t`, `3t + 1`, `3t + 2`, and corner `3t + k` sits at vertex `triangles[t][k]`.
//! The table stores, per corner, the **opposite corner** — the corner of the
//! adjacent triangle across the edge that does *not* touch this corner — plus a
//! representative corner per vertex. Together they answer "which triangle is
//! across this edge?" and "give me a triangle at this vertex" in O(1), which is
//! what mesh walks (contour chaining, region growing) need.
//!
//! Derived, lazily built ([`std::sync::OnceLock`] on the shell), and **never
//! serialized** — a persisted shell rebuilds it on first use.

/// Sentinel for "no opposite corner": the edge is on the mesh boundary.
pub const NO_CORNER: u32 = u32::MAX;

/// Opposite-corner table + vertex→representative-corner map for a triangle mesh.
#[derive(Debug, Clone)]
pub struct CornerTable {
    /// Per corner (`3 * n_triangles` entries): the opposite corner in the
    /// adjacent triangle, or [`NO_CORNER`] on the boundary.
    opposite: Vec<u32>,
    /// Per vertex: one corner incident to it, or [`NO_CORNER`] if no triangle
    /// uses the vertex.
    vertex_corner: Vec<u32>,
}

impl CornerTable {
    /// Build the table. Errors (as a message) when any undirected edge is
    /// carried by more than two triangles — such a mesh is not a surface and
    /// no walk over it is well-defined.
    pub(crate) fn build(n_nodes: usize, triangles: &[[u32; 3]]) -> Result<CornerTable, String> {
        let n_corners = triangles.len() * 3;
        let mut opposite = vec![NO_CORNER; n_corners];
        let mut vertex_corner = vec![NO_CORNER; n_nodes];

        // The edge opposite corner `3t + k` connects the triangle's other two
        // vertices. Key by the undirected vertex pair.
        let mut by_edge: std::collections::HashMap<(u32, u32), Vec<u32>> =
            std::collections::HashMap::new();
        for (t, tri) in triangles.iter().enumerate() {
            for k in 0..3 {
                let corner = (3 * t + k) as u32;
                let v = tri[k] as usize;
                if v >= n_nodes {
                    return Err(format!(
                        "triangle {t} references node {v} outside the shell's {n_nodes} nodes"
                    ));
                }
                if vertex_corner[v] == NO_CORNER {
                    vertex_corner[v] = corner;
                }
                let (a, b) = (tri[(k + 1) % 3], tri[(k + 2) % 3]);
                let key = if a <= b { (a, b) } else { (b, a) };
                by_edge.entry(key).or_default().push(corner);
            }
        }

        for ((a, b), corners) in &by_edge {
            match corners.as_slice() {
                [_] => {}
                [c1, c2] => {
                    opposite[*c1 as usize] = *c2;
                    opposite[*c2 as usize] = *c1;
                }
                more => {
                    return Err(format!(
                        "edge ({a}, {b}) is carried by {} triangles; a shell must be \
                         edge-manifold (at most two)",
                        more.len()
                    ));
                }
            }
        }

        Ok(CornerTable {
            opposite,
            vertex_corner,
        })
    }

    /// The opposite corner of `corner`, or [`NO_CORNER`] on the boundary.
    pub fn opposite(&self, corner: u32) -> u32 {
        self.opposite[corner as usize]
    }

    /// A representative corner at `vertex`, or [`NO_CORNER`] if unused.
    pub fn vertex_corner(&self, vertex: u32) -> u32 {
        self.vertex_corner[vertex as usize]
    }

    /// Number of corners (`3 * n_triangles`).
    pub fn n_corners(&self) -> usize {
        self.opposite.len()
    }

    /// The triangle a corner belongs to.
    pub fn triangle_of(corner: u32) -> usize {
        corner as usize / 3
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Two triangles sharing edge (1, 2): [0,1,2] and [2,1,3].
    fn quad() -> Vec<[u32; 3]> {
        vec![[0, 1, 2], [2, 1, 3]]
    }

    #[test]
    fn opposites_pair_across_the_shared_edge() {
        let ct = CornerTable::build(4, &quad()).unwrap();
        // Corner 0 (vertex 0 of tri 0) is opposite the shared edge (1,2); in
        // tri 1 the corner not on (1,2) is corner 5 (vertex 3).
        assert_eq!(ct.opposite(0), 5);
        assert_eq!(ct.opposite(5), 0);
        // All other corners face boundary edges.
        for c in [1u32, 2, 3, 4] {
            assert_eq!(ct.opposite(c), NO_CORNER, "corner {c} is on the boundary");
        }
        assert_eq!(ct.n_corners(), 6);
    }

    #[test]
    fn vertex_corners_are_incident() {
        let tris = quad();
        let ct = CornerTable::build(4, &tris).unwrap();
        for v in 0..4u32 {
            let c = ct.vertex_corner(v);
            assert_ne!(c, NO_CORNER);
            let t = CornerTable::triangle_of(c);
            assert_eq!(tris[t][c as usize % 3], v);
        }
    }

    #[test]
    fn rejects_a_non_manifold_edge() {
        // Three triangles all carrying edge (0, 1).
        let tris = vec![[0, 1, 2], [1, 0, 3], [0, 1, 4]];
        assert!(CornerTable::build(5, &tris).is_err());
    }

    #[test]
    fn rejects_an_out_of_range_node() {
        assert!(CornerTable::build(2, &quad()).is_err());
    }
}
