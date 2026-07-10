//! Iso-lines and value layers — the property-view surface every level shares.
//!
//! Every surface level (rigid grid, structured mesh, tri mesh) can present any
//! of its property lanes as a **trimesh view**: nodes + CCW triangles + one
//! value per node. Levels 1 and 2 quad-split their cells through their shells
//! (consistent diagonal); level 3 *is* a trimesh. On that view live:
//!
//! * [`iso_lines`](Surface::iso_lines) — NaN-aware marching-triangles contour
//!   extraction (the kernel is `algorithms::surfaces::contour_trimesh`).
//! * [`value_layer`](Surface::value_layer) — the [`ValueLayer`] bundle the
//!   petektools viewer renders (`kind = "trimesh"`).

use crate::algorithms::surfaces::{aligned_levels, contour_trimesh, douglas_peucker};
use crate::core::shell::MeshShell;
use crate::core::{StructuredMeshSurface, Surface, TriSurface};
use crate::foundation::{GeoError, Result};
use ndarray::Array2;

/// One `(level, polylines)` pair per contour level; each polyline is a list of
/// `[x, y]` vertices.
pub type IsoLines = Vec<(f64, Vec<Vec<[f64; 2]>>)>;

/// A property lane presented on a trimesh — the exact bundle the petektools
/// viewer consumes (`kind = "trimesh"`). `values` are per-node, `NaN` allowed;
/// `range` is the finite min/max (`[NaN, NaN]` when no value is finite).
pub struct ValueLayer {
    /// The lane's name (an attribute name, or [`ValueLayer::PRIMARY`]).
    pub name: String,
    /// Node XY.
    pub nodes: Vec<[f64; 2]>,
    /// CCW triangles indexing `nodes`.
    pub triangles: Vec<[u32; 3]>,
    /// One value per node; `NaN` = undefined.
    pub values: Vec<f64>,
    /// `[finite min, finite max]`.
    pub range: [f64; 2],
}

impl ValueLayer {
    /// The layer kind tag the viewer dispatches on.
    pub const KIND: &'static str = "trimesh";
    /// The primary lane's name (attributes use their own names).
    pub const PRIMARY: &'static str = "values";
}

/// Resolve the contour levels: explicit `levels` win; otherwise `interval`
/// produces levels aligned to its multiples spanning the finite value range.
fn resolve_levels(
    values: &[f64],
    interval: Option<f64>,
    levels: Option<Vec<f64>>,
) -> Result<Vec<f64>> {
    if let Some(levels) = levels {
        return Ok(levels);
    }
    let Some(interval) = interval else {
        return Err(GeoError::OutOfRange(
            "iso_lines: provide an interval or explicit levels".into(),
        ));
    };
    let [lo, hi] = finite_range(values);
    aligned_levels(lo, hi, interval)
}

fn finite_range(values: &[f64]) -> [f64; 2] {
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    for &v in values {
        if v.is_finite() {
            lo = lo.min(v);
            hi = hi.max(v);
        }
    }
    if lo > hi {
        [f64::NAN, f64::NAN]
    } else {
        [lo, hi]
    }
}

/// Map a grid-shaped lane onto a mesh shell's nodes through the walk labels
/// (`(0, i, j)` after a quad-split — node identity preserved).
pub(crate) fn grid_lane_on_mesh(shell: &MeshShell, lane: &Array2<f64>) -> Vec<f64> {
    shell
        .labels()
        .iter()
        .map(|l| match l {
            Some((_, i, j)) => lane[[*i as usize, *j as usize]],
            None => f64::NAN,
        })
        .collect()
}

fn build_layer(
    name: String,
    nodes: Vec<[f64; 2]>,
    triangles: Vec<[u32; 3]>,
    values: Vec<f64>,
) -> ValueLayer {
    let range = finite_range(&values);
    ValueLayer {
        name,
        nodes,
        triangles,
        values,
        range,
    }
}

/// Assemble the [`ValueLayer`] from a mesh shell and its per-node lane, applying
/// the display-only LOD `stride` when requested. `None`/`Some(1)` is the full
/// mesh (byte-identical to the pre-LOD behaviour). `Some(k)` decimates each
/// fault block to its `i % k == 0 && j % k == 0` nodes, re-triangulated as the
/// coarse quad-split; node values are the nodes' own values (no averaging — a
/// display LOD, not a resample). The `range` is always the **full-resolution**
/// finite min/max, so a lane's colours stay stable across LODs.
fn build_layer_strided(
    shell: &MeshShell,
    name: String,
    mesh_values: Vec<f64>,
    stride: Option<usize>,
) -> ValueLayer {
    match stride {
        None | Some(0) | Some(1) => build_layer(
            name,
            shell.nodes().to_vec(),
            shell.triangles().to_vec(),
            mesh_values,
        ),
        Some(k) => {
            let (nodes, triangles, orig) = shell.strided_lattice(k);
            let values = orig.iter().map(|&idx| mesh_values[idx]).collect();
            ValueLayer {
                name,
                nodes,
                triangles,
                values,
                range: finite_range(&mesh_values), // stable colours: full-res range
            }
        }
    }
}

/// Simplify each iso-line polyline in place with Douglas–Peucker when a
/// `simplify` tolerance (world units) is given. A no-op otherwise.
fn simplify_iso_lines(mut out: IsoLines, simplify: Option<f64>) -> IsoLines {
    if let Some(tol) = simplify {
        for (_, lines) in out.iter_mut() {
            for line in lines.iter_mut() {
                *line = douglas_peucker(line, tol);
            }
        }
    }
    out
}

impl Surface {
    fn lane(&self, attr: Option<&str>) -> Result<(&Array2<f64>, String)> {
        match attr {
            None => Ok((self.values(), ValueLayer::PRIMARY.to_string())),
            Some(name) => self
                .attr(name)
                .map(|a| (a, name.to_string()))
                .ok_or_else(|| GeoError::NotFound(format!("no attribute layer '{name}'"))),
        }
    }

    /// Iso-lines of a property lane (the primary values, or `attr`). Linear
    /// interpolation per triangle of the quad-split grid (consistent
    /// diagonal), NaN-aware: cells touching an undefined node break the lines
    /// rather than bend them. Explicit `levels` win over `interval` (levels
    /// aligned to multiples of the interval across the value range).
    pub fn iso_lines(
        &self,
        interval: Option<f64>,
        levels: Option<Vec<f64>>,
        attr: Option<&str>,
        simplify: Option<f64>,
    ) -> Result<IsoLines> {
        let shell = self.geom.to_mesh_shell()?;
        let (lane, _) = self.lane(attr)?;
        let values = grid_lane_on_mesh(&shell, lane);
        let levels = resolve_levels(&values, interval, levels)?;
        let out = contour_trimesh(shell.nodes(), shell.triangles(), &values, &levels);
        Ok(simplify_iso_lines(out, simplify))
    }

    /// A property lane as a trimesh [`ValueLayer`] (nodes/triangles from the
    /// quad-split grid, XY computed from the geometry). `stride = Some(k)`
    /// returns the coarse-LOD decimation (see [`build_layer_strided`]).
    pub fn value_layer(&self, attr: Option<&str>, stride: Option<usize>) -> Result<ValueLayer> {
        let shell = self.geom.to_mesh_shell()?;
        let (lane, name) = self.lane(attr)?;
        let values = grid_lane_on_mesh(&shell, lane);
        Ok(build_layer_strided(&shell, name, values, stride))
    }
}

impl StructuredMeshSurface {
    fn lane(&self, attr: Option<&str>) -> Result<(&Array2<f64>, String)> {
        match attr {
            None => Ok((self.values(), ValueLayer::PRIMARY.to_string())),
            Some(name) => self
                .attr(name)
                .map(|a| (a, name.to_string()))
                .ok_or_else(|| GeoError::NotFound(format!("no attribute layer '{name}'"))),
        }
    }

    /// Iso-lines of a property lane. See [`Surface::iso_lines`]; the triangles
    /// come from the shell's quad-split (explicit node XY honoured exactly).
    pub fn iso_lines(
        &self,
        interval: Option<f64>,
        levels: Option<Vec<f64>>,
        attr: Option<&str>,
        simplify: Option<f64>,
    ) -> Result<IsoLines> {
        let mesh = self.shell().to_mesh_shell()?;
        let (lane, _) = self.lane(attr)?;
        let values = grid_lane_on_mesh(&mesh, lane);
        let levels = resolve_levels(&values, interval, levels)?;
        let out = contour_trimesh(mesh.nodes(), mesh.triangles(), &values, &levels);
        Ok(simplify_iso_lines(out, simplify))
    }

    /// A property lane as a trimesh [`ValueLayer`] (nodes/triangles from the
    /// shell's quad-split). `stride = Some(k)` returns the coarse-LOD
    /// decimation (see [`build_layer_strided`]).
    pub fn value_layer(&self, attr: Option<&str>, stride: Option<usize>) -> Result<ValueLayer> {
        let mesh = self.shell().to_mesh_shell()?;
        let (lane, name) = self.lane(attr)?;
        let values = grid_lane_on_mesh(&mesh, lane);
        Ok(build_layer_strided(&mesh, name, values, stride))
    }
}

impl TriSurface {
    fn lane(&self, attr: Option<&str>) -> Result<(&[f64], String)> {
        match attr {
            None => Ok((self.values(), ValueLayer::PRIMARY.to_string())),
            Some(name) => self
                .attr(name)
                .map(|a| (a, name.to_string()))
                .ok_or_else(|| GeoError::NotFound(format!("no attribute layer '{name}'"))),
        }
    }

    /// Iso-lines of a property lane, contoured per shell triangle with
    /// per-node values. See [`Surface::iso_lines`].
    pub fn iso_lines(
        &self,
        interval: Option<f64>,
        levels: Option<Vec<f64>>,
        attr: Option<&str>,
        simplify: Option<f64>,
    ) -> Result<IsoLines> {
        let (values, _) = self.lane(attr)?;
        let levels = resolve_levels(values, interval, levels)?;
        let out = contour_trimesh(
            self.shell().nodes(),
            self.shell().triangles(),
            values,
            &levels,
        );
        Ok(simplify_iso_lines(out, simplify))
    }

    /// A property lane as a trimesh [`ValueLayer`] — the shell's own nodes
    /// and triangles. `stride = Some(k)` returns the coarse-LOD decimation
    /// (see [`build_layer_strided`]).
    pub fn value_layer(&self, attr: Option<&str>, stride: Option<usize>) -> Result<ValueLayer> {
        let (values, name) = self.lane(attr)?;
        Ok(build_layer_strided(
            self.shell(),
            name,
            values.to_vec(),
            stride,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::foundation::GridGeometry;
    use approx::assert_relative_eq;
    use ndarray::Array2;

    /// An 11 x 5 axis-aligned grid, 10 m spacing, z = 2x + 100 (a tilted plane).
    fn tilted_plane() -> Surface {
        let geom = GridGeometry {
            xori: 0.0,
            yori: 0.0,
            xinc: 10.0,
            yinc: 10.0,
            ncol: 11,
            nrow: 5,
            rotation_deg: 0.0,
            yflip: false,
        };
        let mut v = Array2::zeros((11, 5));
        for j in 0..5 {
            for i in 0..11 {
                let (x, _) = geom.node_xy(i, j);
                v[[i, j]] = 2.0 * x + 100.0;
            }
        }
        Surface::new(geom, v).unwrap()
    }

    #[test]
    fn tilted_plane_gives_straight_iso_lines_at_exact_x() {
        let s = tilted_plane();
        let out = s.iso_lines(Some(50.0), None, None, None).unwrap();
        // Values span [100, 300] → levels 100, 150, 200, 250, 300.
        let levels: Vec<f64> = out.iter().map(|(l, _)| *l).collect();
        assert_eq!(levels, vec![100.0, 150.0, 200.0, 250.0, 300.0]);
        for (level, lines) in &out {
            if *level <= 100.0 {
                continue; // at the exact minimum every node is >= level: no crossing
            }
            let expect_x = (level - 100.0) / 2.0;
            assert_eq!(lines.len(), 1, "one straight line per level {level}");
            let mut ymin = f64::INFINITY;
            let mut ymax = f64::NEG_INFINITY;
            for p in &lines[0] {
                assert_relative_eq!(p[0], expect_x, epsilon = 1e-9);
                ymin = ymin.min(p[1]);
                ymax = ymax.max(p[1]);
            }
            assert_relative_eq!(ymin, 0.0, epsilon = 1e-9);
            assert_relative_eq!(ymax, 40.0, epsilon = 1e-9);
        }
    }

    #[test]
    fn a_nan_hole_breaks_the_line_not_bends_it() {
        let mut s = tilted_plane();
        // Hole at node (5, 2): x = 50, mid-row. The 200-level runs at x = 50 —
        // straight through the hole's cells.
        let mut v = s.values().clone();
        v[[5, 2]] = f64::NAN;
        s = Surface::new(s.geom.clone(), v).unwrap();
        let out = s.iso_lines(None, Some(vec![200.0]), None, None).unwrap();
        let lines = &out[0].1;
        assert!(
            lines.len() >= 2,
            "the hole must break the line into pieces, got {}",
            lines.len()
        );
        for line in lines {
            for p in line {
                assert_relative_eq!(p[0], 50.0, epsilon = 1e-9); // never bent
            }
        }
        // The y-band around the hole is empty: the four cells touching row j=2
        // at the hole's columns are skipped whole.
        let ys: Vec<f64> = lines.iter().flatten().map(|p| p[1]).collect();
        assert!(ys.iter().all(|&y| !(10.0 + 1e-9 < y && y < 30.0 - 1e-9)));
    }

    #[test]
    fn explicit_levels_win_over_interval() {
        let s = tilted_plane();
        let out = s
            .iso_lines(Some(50.0), Some(vec![137.0, 253.0]), None, None)
            .unwrap();
        let levels: Vec<f64> = out.iter().map(|(l, _)| *l).collect();
        assert_eq!(levels, vec![137.0, 253.0]);
        assert!(
            s.iso_lines(None, None, None, None).is_err(),
            "no interval, no levels"
        );
    }

    #[test]
    fn attr_lane_is_contoured_when_named() {
        let mut s = tilted_plane();
        // The attribute plane runs the other way: value = y.
        let mut a = Array2::zeros((11, 5));
        for j in 0..5 {
            for i in 0..11 {
                let (_, y) = s.geom.node_xy(i, j);
                a[[i, j]] = y;
            }
        }
        s.set_attr("twt", a).unwrap();
        let out = s
            .iso_lines(None, Some(vec![25.0]), Some("twt"), None)
            .unwrap();
        let lines = &out[0].1;
        assert_eq!(lines.len(), 1);
        for p in &lines[0] {
            assert_relative_eq!(p[1], 25.0, epsilon = 1e-9);
        }
        assert!(s.iso_lines(Some(1.0), None, Some("missing"), None).is_err());
    }

    #[test]
    fn value_layer_is_the_viewer_trimesh_bundle() {
        let mut s = tilted_plane();
        let mut v = s.values().clone();
        for j in 0..5 {
            v[[10, j]] = f64::NAN; // NaN out the whole max column (value 300)
        }
        s = Surface::new(s.geom.clone(), v).unwrap();
        let layer = s.value_layer(None, None).unwrap();
        assert_eq!(ValueLayer::KIND, "trimesh");
        assert_eq!(layer.name, "values");
        assert_eq!(layer.nodes.len(), 11 * 5);
        assert_eq!(layer.triangles.len(), 2 * 10 * 4);
        assert_eq!(layer.values.len(), layer.nodes.len());
        assert!(layer.values.iter().any(|v| v.is_nan()), "NaN is allowed");
        // range = FINITE min/max: the NaN'd column (300) is excluded.
        assert_relative_eq!(layer.range[0], 100.0, epsilon = 1e-9);
        assert_relative_eq!(layer.range[1], 280.0, epsilon = 1e-9);
        // Per-node values match the lane through the labels.
        for (k, node) in layer.nodes.iter().enumerate() {
            let expect = 2.0 * node[0] + 100.0;
            let got = layer.values[k];
            if got.is_finite() {
                assert_relative_eq!(got, expect, epsilon = 1e-9);
            }
        }
    }

    /// A wider flat lattice (z = 2x + 100) for the LOD tests.
    fn wide_plane(ncol: usize, nrow: usize) -> Surface {
        let geom = GridGeometry {
            xori: 0.0,
            yori: 0.0,
            xinc: 10.0,
            yinc: 10.0,
            ncol,
            nrow,
            rotation_deg: 0.0,
            yflip: false,
        };
        let mut v = Array2::zeros((ncol, nrow));
        for j in 0..nrow {
            for i in 0..ncol {
                let (x, _) = geom.node_xy(i, j);
                v[[i, j]] = 2.0 * x + 100.0;
            }
        }
        Surface::new(geom, v).unwrap()
    }

    #[test]
    fn value_layer_stride_is_the_coarse_quad_triangulation() {
        // 9 x 7 flat lattice; stride 2 keeps the i,j-even nodes, re-triangulated
        // as the coarse quad-split. Node count / triangles match the ceil grid.
        let s = wide_plane(9, 7);
        let full = s.value_layer(None, None).unwrap();
        assert_eq!(
            s.value_layer(None, Some(1)).unwrap().nodes.len(),
            full.nodes.len()
        );

        let even = |n: usize| (0..n).filter(|i| i % 2 == 0).count();
        let (nc, nr) = (even(9), even(7)); // 5 x 4 coarse nodes
        let lod = s.value_layer(None, Some(2)).unwrap();
        assert_eq!(lod.nodes.len(), nc * nr);
        assert_eq!(lod.triangles.len(), 2 * (nc - 1) * (nr - 1));
        // Fewer triangles than full — the whole point.
        assert!(lod.triangles.len() < full.triangles.len());
        // Range comes from the FULL-resolution lane (stable colours).
        assert_eq!(lod.range, full.range);
        // Values are the nodes' own values (no averaging) — the plane holds.
        for (node, &val) in lod.nodes.iter().zip(&lod.values) {
            assert_relative_eq!(val, 2.0 * node[0] + 100.0, epsilon = 1e-9);
        }
    }

    #[test]
    fn value_layer_stride_keeps_faulted_blocks_separate() {
        // Two blocks → the coarse layer must still honour the fault (both
        // blocks decimated independently, no bridge triangle across the seam).
        let mut coords = Vec::new();
        for j in 0..12 {
            for i in 0..8 {
                coords.push([50.0 * i as f64, 50.0 * j as f64, -1800.0]);
            }
        }
        for j in 0..12 {
            for i in 10..18 {
                coords.push([50.0 * i as f64 + 20.0, 50.0 * j as f64 + 25.0, -1900.0]);
            }
        }
        let tin = crate::core::PointSet::from_coords(coords)
            .to_tri_surface(None, None)
            .unwrap();
        assert_eq!(tin.components(), 2);
        let full = tin.value_layer(None, None).unwrap();
        let lod = tin.value_layer(None, Some(2)).unwrap();
        assert!(lod.triangles.len() < full.triangles.len());
        assert!(!lod.nodes.is_empty());
        assert_eq!(lod.range, full.range, "range from the full-res lane");
        // No coarse triangle spans the fault: the seam gap (~170 m) exceeds any
        // legitimate coarse in-block edge (a 100 m cell's diagonal ≈ 141 m), so
        // every triangle edge staying under 160 m proves the blocks never merged.
        for t in &lod.triangles {
            for &(a, b) in &[(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                let (p, q) = (lod.nodes[a as usize], lod.nodes[b as usize]);
                let len = ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2)).sqrt();
                assert!(len < 160.0, "a coarse triangle bridged the fault: {len} m");
            }
        }
    }

    #[test]
    fn iso_lines_simplify_collapses_a_straight_contour() {
        // The tilted plane's contours are straight; DP with a small tolerance
        // collapses each to its two endpoints without moving them.
        let s = wide_plane(21, 5);
        let full = s.iso_lines(None, Some(vec![200.0]), None, None).unwrap();
        let simp = s
            .iso_lines(None, Some(vec![200.0]), None, Some(1.0))
            .unwrap();
        let (lf, ls) = (&full[0].1, &simp[0].1);
        assert_eq!(lf.len(), ls.len(), "same number of polylines");
        for (a, b) in lf.iter().zip(ls) {
            assert!(b.len() <= a.len(), "simplify never adds points");
            assert_eq!(b.first(), a.first(), "endpoints preserved");
            assert_eq!(b.last(), a.last());
            // A straight contour collapses to exactly its two endpoints.
            assert_eq!(b.len(), 2);
        }
    }
}
