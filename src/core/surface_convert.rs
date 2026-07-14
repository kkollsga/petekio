//! Cross-level surface conversions.
//!
//! **Up is free and lossless** (node identity preserved; every attribute lane
//! carries over 1:1): `Surface → StructuredMeshSurface → TriSurface`.
//! **Down is a resample** onto a target [`GridGeometry`] through the shared
//! gridding kernels (the same path `to_points().to_surface(...)` takes), one
//! lane at a time — primary plus all attributes.
//!
//! The shell-level counterparts (`GridGeometry::to_structured_shell` /
//! `to_mesh_shell`, `StructuredShell::to_mesh_shell`, `infer_grid`) live in
//! [`core::shell`](crate::core::shell); this module lifts them to full
//! surfaces (shell + property lanes).

use crate::core::value_layer::grid_lane_on_mesh;
use crate::core::{
    AttributeMetadata, GridMethod, PointSet, StructuredMeshSurface, Surface, TriSurface,
};
use crate::foundation::{GridGeometry, HasHistory, Result};
use std::sync::Arc;

impl Surface {
    /// Lift to a level-2 [`StructuredMeshSurface`]: explicit per-node XY
    /// computed from the grid, `nominal_geometry` = this grid, all attribute
    /// lanes carried 1:1. Lossless.
    pub fn to_structured_mesh(&self) -> Result<StructuredMeshSurface> {
        let shell = Arc::new(self.geom.to_structured_shell());
        let mut out = StructuredMeshSurface::from_shell(shell, self.values().clone())?;
        out.set_primary_metadata(self.primary_metadata().cloned());
        for name in self.attr_names() {
            let lane = self.attr(name).expect("listed attribute exists").clone();
            let metadata = self
                .attr_metadata(name)
                .expect("listed attribute has metadata")
                .clone();
            out.set_attr_with_metadata(name, lane, metadata)?;
        }
        out.set_history(self.history_with("surface.to_structured_mesh()"));
        Ok(out)
    }

    /// Lift to a level-3 [`TriSurface`]: the grid quad-splits along a
    /// consistent diagonal, every lane maps per node through the shell's
    /// `(i, j)` labels. Lossless (node identity preserved).
    pub fn to_tri_surface(&self) -> Result<TriSurface> {
        let shell = Arc::new(self.geom.to_mesh_shell()?);
        let values = grid_lane_on_mesh(&shell, self.values());
        let mut out = TriSurface::from_shell(Arc::clone(&shell), values)?;
        out.set_primary_metadata(self.primary_metadata().cloned());
        for name in self.attr_names() {
            let lane = grid_lane_on_mesh(&shell, self.attr(name).expect("listed attribute exists"));
            let metadata = self
                .attr_metadata(name)
                .expect("listed attribute has metadata")
                .clone();
            out.set_attr_with_metadata(name, lane, metadata)?;
        }
        out.set_history(self.history_with("surface.to_tri_surface()"));
        Ok(out)
    }
}

impl StructuredMeshSurface {
    /// Lift to a level-3 [`TriSurface`]: the shell quad-splits (explicit XY
    /// honoured exactly), every lane maps per node through the `(i, j)`
    /// labels. Lossless for every node that carries finite XY.
    pub fn to_tri_surface(&self) -> Result<TriSurface> {
        let mesh = Arc::new(self.shell().to_mesh_shell()?);
        let values = grid_lane_on_mesh(&mesh, self.values());
        let mut out = TriSurface::from_shell(Arc::clone(&mesh), values)?;
        out.set_primary_metadata(self.primary_metadata().cloned());
        for name in self.attr_names() {
            let lane = grid_lane_on_mesh(&mesh, self.attr(name).expect("listed attribute exists"));
            let metadata = self
                .attr_metadata(name)
                .expect("listed attribute has metadata")
                .clone();
            out.set_attr_with_metadata(name, lane, metadata)?;
        }
        out.set_history(
            self.operation_history()
                .with_entry("structured_surface.to_tri_surface()"),
        );
        Ok(out)
    }

    /// Resample primary **and all attribute lanes** onto a target regular
    /// geometry through the shared gridding kernels (the lossy downward
    /// conversion; same kernels as [`PointSet::to_surface`]).
    pub fn resample(&self, target: &GridGeometry, method: GridMethod) -> Result<Surface> {
        let (x, y) = (self.x(), self.y());
        let lane_coords = |lane: &ndarray::Array2<f64>| -> Vec<[f64; 3]> {
            let mut coords = Vec::new();
            for j in 0..self.nrow() {
                for i in 0..self.ncol() {
                    let (px, py, pz) = (x[[i, j]], y[[i, j]], lane[[i, j]]);
                    if px.is_finite() && py.is_finite() && pz.is_finite() {
                        coords.push([px, py, pz]);
                    }
                }
            }
            coords
        };
        let lanes: Vec<(AttributeMetadata, Vec<[f64; 3]>)> = self
            .attr_names()
            .into_iter()
            .map(|name| {
                (
                    self.attr_metadata(name).expect("listed metadata").clone(),
                    lane_coords(self.attr(name).expect("listed")),
                )
            })
            .collect();
        let mut out = grid_lanes(
            lane_coords(self.values()),
            self.primary_metadata(),
            lanes,
            target,
            method,
        )?;
        out.set_primary_metadata(self.primary_metadata().cloned());
        out.set_history(self.operation_history().with_entry(format!(
            "structured_surface.resample(ncol={}, nrow={}, method={method:?})",
            target.ncol, target.nrow
        )));
        Ok(out)
    }
}

impl TriSurface {
    /// Resample primary **and all attribute lanes** onto a target regular
    /// geometry through the shared gridding kernels (the lossy downward
    /// conversion; same kernels as [`PointSet::to_surface`]).
    pub fn resample(&self, target: &GridGeometry, method: GridMethod) -> Result<Surface> {
        let nodes = self.shell().nodes();
        let lane_coords = |lane: &[f64]| -> Vec<[f64; 3]> {
            nodes
                .iter()
                .zip(lane)
                .filter(|(_, z)| z.is_finite())
                .map(|(n, z)| [n[0], n[1], *z])
                .collect()
        };
        let lanes: Vec<(AttributeMetadata, Vec<[f64; 3]>)> = self
            .attr_names()
            .into_iter()
            .map(|name| {
                (
                    self.attr_metadata(name).expect("listed metadata").clone(),
                    lane_coords(self.attr(name).expect("listed")),
                )
            })
            .collect();
        let mut out = grid_lanes(
            lane_coords(self.values()),
            self.primary_metadata(),
            lanes,
            target,
            method,
        )?;
        out.set_primary_metadata(self.primary_metadata().cloned());
        out.set_history(self.operation_history().with_entry(format!(
            "tri_surface.resample(ncol={}, nrow={}, method={method:?})",
            target.ncol, target.nrow
        )));
        Ok(out)
    }
}

/// Grid the primary lane onto `target`, then each attribute lane, reusing the
/// existing gridding kernels through [`PointSet::to_surface`].
fn grid_lanes(
    primary: Vec<[f64; 3]>,
    primary_metadata: Option<&AttributeMetadata>,
    lanes: Vec<(AttributeMetadata, Vec<[f64; 3]>)>,
    target: &GridGeometry,
    method: GridMethod,
) -> Result<Surface> {
    let primary_method = match primary_metadata.map(|metadata| metadata.kind) {
        Some(crate::AttributeKind::Categorical) => GridMethod::Nearest,
        _ => method,
    };
    let mut out = PointSet::from_coords(primary).to_surface(target.clone(), primary_method)?;
    for (metadata, coords) in lanes {
        let lane_method = match metadata.kind {
            crate::AttributeKind::Continuous => method,
            crate::AttributeKind::Categorical => GridMethod::Nearest,
        };
        let lane = PointSet::from_coords(coords)
            .to_surface(target.clone(), lane_method)?
            .values()
            .clone();
        let name = metadata.id.clone();
        out.set_attr_with_metadata(&name, lane, metadata)?;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;
    use ndarray::Array2;

    fn geom() -> GridGeometry {
        GridGeometry {
            xori: 100.0,
            yori: 200.0,
            xinc: 10.0,
            yinc: 10.0,
            ncol: 6,
            nrow: 5,
            rotation_deg: 0.0,
            yflip: false,
        }
    }

    /// z = x + 2y; attribute "amp" = x - y (both affine → exact under bilinear
    /// and preserved exactly under node-identity lifts).
    fn surface() -> Surface {
        let g = geom();
        let mut v = Array2::zeros((g.ncol, g.nrow));
        let mut a = Array2::zeros((g.ncol, g.nrow));
        for j in 0..g.nrow {
            for i in 0..g.ncol {
                let (x, y) = g.node_xy(i, j);
                v[[i, j]] = x + 2.0 * y;
                a[[i, j]] = x - y;
            }
        }
        let mut s = Surface::new(g, v).unwrap();
        s.set_attr_with_metadata(
            "amp",
            a,
            crate::AttributeMetadata::new(
                "amp",
                "Amplitude",
                crate::AttributeKind::Continuous,
                Some("mV".into()),
                None,
            )
            .unwrap(),
        )
        .unwrap();
        s
    }

    #[test]
    fn upward_conversions_carry_all_attributes_node_for_node() {
        let s = surface();
        let g = geom();

        // Level 1 → 2: values and attrs are bit-identical per (i, j).
        let sm = s.to_structured_mesh().unwrap();
        assert_eq!(sm.values(), s.values());
        assert_eq!(sm.attr("amp").unwrap(), s.attr("amp").unwrap());
        assert_eq!(sm.attr_metadata("amp"), s.attr_metadata("amp"));
        assert_eq!(sm.nominal_geometry(), Some(&g));

        // Level 1 → 3 and 2 → 3 agree, and every node keeps its lane values.
        for tri in [s.to_tri_surface().unwrap(), sm.to_tri_surface().unwrap()] {
            assert_eq!(tri.points().len(), g.ncol * g.nrow);
            assert_eq!(tri.attr_names(), vec!["amp"]);
            assert_eq!(tri.attr_metadata("amp"), s.attr_metadata("amp"));
            for (k, p) in tri.points().iter().enumerate() {
                assert_relative_eq!(p[2], p[0] + 2.0 * p[1], epsilon = 1e-9);
                assert_relative_eq!(tri.attr("amp").unwrap()[k], p[0] - p[1], epsilon = 1e-9);
            }
        }
    }

    #[test]
    fn nan_values_survive_the_lift_as_properties_not_holes() {
        // The shell is value-independent: a NaN node stays a node.
        let mut v = surface().values().clone();
        v[[2, 2]] = f64::NAN;
        let mut s = Surface::new(geom(), v).unwrap();
        s.set_attr("amp", surface().attr("amp").unwrap().clone())
            .unwrap();
        let tri = s.to_tri_surface().unwrap();
        assert_eq!(tri.points().len(), geom().ncol * geom().nrow);
        assert_eq!(tri.values().iter().filter(|z| z.is_nan()).count(), 1);
        // The attribute at that node is still defined.
        assert!(tri.attr("amp").unwrap().iter().all(|a| a.is_finite()));
    }

    #[test]
    fn downward_resample_carries_primary_and_attributes() {
        let s = surface();
        // A sub-lattice of the source: every target node coincides with a data
        // node, so `Nearest` must reproduce both lanes exactly.
        let target = GridGeometry {
            xori: 110.0,
            yori: 210.0,
            xinc: 10.0,
            yinc: 10.0,
            ncol: 3,
            nrow: 3,
            rotation_deg: 0.0,
            yflip: false,
        };

        let sm = s.to_structured_mesh().unwrap();
        let tri = s.to_tri_surface().unwrap();
        for down in [
            sm.resample(&target, GridMethod::Nearest).unwrap(),
            tri.resample(&target, GridMethod::Nearest).unwrap(),
        ] {
            assert_eq!(down.geom, target);
            assert_eq!(down.attr_names(), vec!["amp"]);
            assert_eq!(down.attr_metadata("amp"), s.attr_metadata("amp"));
            for j in 0..target.nrow {
                for i in 0..target.ncol {
                    let (x, y) = target.node_xy(i, j);
                    assert_relative_eq!(down.values()[[i, j]], x + 2.0 * y, epsilon = 1e-9);
                    assert_relative_eq!(down.attr("amp").unwrap()[[i, j]], x - y, epsilon = 1e-9);
                }
            }
        }
    }

    #[test]
    fn round_trip_grid_to_tri_and_back_via_infer_grid() {
        let s = surface();
        let tri = s.to_tri_surface().unwrap();
        let g = tri.infer_grid(1e-6).unwrap();
        assert_relative_eq!(g.xori, s.geom.xori, epsilon = 1e-6);
        assert_relative_eq!(g.xinc, s.geom.xinc, epsilon = 1e-9);
        assert_eq!((g.ncol, g.nrow), (s.geom.ncol, s.geom.nrow));

        let sm = s.to_structured_mesh().unwrap();
        let g2 = sm.infer_grid(1e-6).unwrap();
        assert_eq!((g2.ncol, g2.nrow), (s.geom.ncol, s.geom.nrow));
    }

    #[test]
    fn categorical_lanes_use_nearest_while_continuous_lanes_use_requested_method() {
        let mut s = surface();
        let mut facies = Array2::zeros((geom().ncol, geom().nrow));
        for j in 0..geom().nrow {
            for i in 0..geom().ncol {
                facies[[i, j]] = if i < 3 { 1.0 } else { 2.0 };
            }
        }
        s.set_attr_with_metadata(
            "facies",
            facies,
            crate::AttributeMetadata::new(
                "facies",
                "Facies",
                crate::AttributeKind::Categorical,
                None,
                None,
            )
            .unwrap(),
        )
        .unwrap();
        let target = GridGeometry {
            xori: 105.0,
            yori: 205.0,
            xinc: 10.0,
            yinc: 10.0,
            ncol: 5,
            nrow: 4,
            rotation_deg: 0.0,
            yflip: false,
        };
        let structured = s.to_structured_mesh().unwrap();
        let tri = s.to_tri_surface().unwrap();
        for down in [
            structured
                .resample(&target, GridMethod::InverseDistance)
                .unwrap(),
            tri.resample(&target, GridMethod::InverseDistance).unwrap(),
        ] {
            assert!(down
                .attr("facies")
                .unwrap()
                .iter()
                .filter(|value| value.is_finite())
                .all(|value| value.fract() == 0.0));
            assert_eq!(
                down.attr_metadata("facies").unwrap().kind,
                crate::AttributeKind::Categorical
            );
            assert!(down
                .attr("amp")
                .unwrap()
                .iter()
                .any(|value| value.is_finite() && value.fract() != 0.0));
        }
        for promoted in [
            structured
                .as_attr_surface("facies")
                .unwrap()
                .resample(&target, GridMethod::InverseDistance)
                .unwrap(),
            tri.as_attr_surface("facies")
                .unwrap()
                .resample(&target, GridMethod::InverseDistance)
                .unwrap(),
        ] {
            assert!(promoted
                .values()
                .iter()
                .filter(|value| value.is_finite())
                .all(|value| value.fract() == 0.0));
            assert_eq!(
                promoted.primary_metadata().unwrap().kind,
                crate::AttributeKind::Categorical
            );
        }
    }
}
