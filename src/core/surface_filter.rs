//! Surface filtering and outline ops: NaN-aware smoothing and the boundary
//! polygon of the defined region. Like the arithmetic ops, these return **new**
//! values and respect the `NaN` (undefined) convention.

use super::{GridMethod, PolygonSet, Surface};
use crate::algorithms::surfaces::dip_fields;
use crate::foundation::{GeoError, Result};
use ndarray::Array2;

impl Surface {
    /// NaN-aware moving-average smoothing with a Chebyshev (square) window of the
    /// given `radius`. Each **defined** node is replaced by the mean of the
    /// defined nodes in its `(2·radius+1)²` window; undefined (`NaN`) nodes stay
    /// undefined, so the defined mask is preserved (smoothing never grows the
    /// surface). `radius == 0` is the identity. Returns a new surface.
    pub fn smooth(&self, radius: usize) -> Surface {
        let (nc, nr) = (self.geom.ncol, self.geom.nrow);
        let v = self.values();
        let mut out = Array2::from_elem((nc, nr), f64::NAN);
        for j in 0..nr {
            for i in 0..nc {
                if v[[i, j]].is_nan() {
                    continue; // preserve the undefined mask
                }
                let mut sum = 0.0;
                let mut count = 0usize;
                let (i0, i1) = (
                    i.saturating_sub(radius),
                    i.saturating_add(radius).min(nc - 1),
                );
                let (j0, j1) = (
                    j.saturating_sub(radius),
                    j.saturating_add(radius).min(nr - 1),
                );
                for jj in j0..=j1 {
                    for ii in i0..=i1 {
                        let val = v[[ii, jj]];
                        if !val.is_nan() {
                            sum += val;
                            count += 1;
                        }
                    }
                }
                if count > 0 {
                    let mean = sum / count as f64;
                    // A +∞/-∞ mixture has no arithmetic mean; retain the
                    // original defined node so smoothing never grows the NaN mask.
                    out[[i, j]] = if mean.is_nan() { v[[i, j]] } else { mean };
                }
            }
        }
        let mut surface = Surface::from_values_unchecked(self.geom.clone(), out);
        surface.set_history(self.history_with(format!("surface.smooth(radius={radius})")));
        surface
    }

    /// Geological dip angle in degrees (`0` = flat, approaching `90` =
    /// vertical), calculated from NaN-aware finite differences in world space.
    pub fn dip_angle(&self) -> Surface {
        self.dip_surfaces().0
    }

    /// Down-dip azimuth in degrees clockwise from North. Flat nodes have no
    /// unique direction and are therefore `NaN`.
    pub fn dip_azimuth(&self) -> Surface {
        self.dip_surfaces().1
    }

    fn dip_surfaces(&self) -> (Surface, Surface) {
        let (angle, azimuth) = dip_fields(
            self.values(),
            self.geom.xinc,
            self.geom.yinc * self.geom.yflip_factor(),
            self.geom.rotation_deg,
        );
        let mut angle_surface = Surface::from_values_unchecked(self.geom.clone(), angle);
        angle_surface.set_history(self.history_with("surface.dip_angle()"));
        let mut azimuth_surface = Surface::from_values_unchecked(self.geom.clone(), azimuth);
        azimuth_surface.set_history(self.history_with("surface.dip_azimuth()"));
        (angle_surface, azimuth_surface)
    }

    /// Fill only the primary layer's original `NaN` nodes by gridding its
    /// finite nodes over the same lattice. Existing non-NaN values, including
    /// infinities, are copied back bit-for-bit. Attribute lanes are not carried.
    pub fn extrapolate(&self, method: GridMethod) -> Result<Surface> {
        let has_holes = self.values().iter().any(|v| v.is_nan());
        if !has_holes {
            let mut out = Surface::from_values_unchecked(self.geom.clone(), self.values().clone());
            out.set_history(self.history_with(format!("surface.extrapolate(method={method:?})")));
            return Ok(out);
        }

        let mut controls = Vec::new();
        for j in 0..self.geom.nrow {
            for i in 0..self.geom.ncol {
                let z = self.values()[[i, j]];
                if z.is_finite() {
                    let (x, y) = self.geom.node_xy(i, j);
                    controls.push([x, y, z]);
                }
            }
        }
        if controls.is_empty() {
            return Err(GeoError::OutOfRange(
                "surface.extrapolate requires at least one finite source node".into(),
            ));
        }

        let filled = petektools::grid(&controls, &self.geom.to_lattice(), method.to_petektools())?;
        let mut values = self.values().clone();
        for ((i, j), value) in values.indexed_iter_mut() {
            if value.is_nan() {
                *value = filled[[i, j]];
            }
        }
        let mut out = Surface::from_values_unchecked(self.geom.clone(), values);
        out.set_history(self.history_with(format!("surface.extrapolate(method={method:?})")));
        Ok(out)
    }

    /// The edge polygon enclosing the surface's defined region — the convex hull
    /// (in world XY) of all non-`NaN` nodes. Returns `None` if fewer than three
    /// defined nodes exist.
    ///
    /// This is a **convex** outline (a drainage-boundary approximation); a
    /// concave hull is a future refinement. Backs `ModelInputs::boundary` when no
    /// explicit boundary polygon is supplied.
    pub fn edge(&self) -> Option<PolygonSet> {
        let (nc, nr) = (self.geom.ncol, self.geom.nrow);
        let v = self.values();
        let mut pts: Vec<[f64; 2]> = Vec::new();
        for j in 0..nr {
            for i in 0..nc {
                if !v[[i, j]].is_nan() {
                    let (x, y) = self.geom.node_xy(i, j);
                    pts.push([x, y]);
                }
            }
        }
        if pts.len() < 3 {
            return None;
        }
        PolygonSet::convex_hull_xy(pts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::foundation::GridGeometry;
    use approx::assert_relative_eq;

    fn geom(n: usize, inc: f64) -> GridGeometry {
        GridGeometry {
            xori: 0.0,
            yori: 0.0,
            xinc: inc,
            yinc: inc,
            ncol: n,
            nrow: n,
            rotation_deg: 0.0,
            yflip: false,
        }
    }

    fn plane(geom: GridGeometry, gx: f64, gy: f64, intercept: f64) -> Surface {
        let mut values = Array2::zeros((geom.ncol, geom.nrow));
        for j in 0..geom.nrow {
            for i in 0..geom.ncol {
                let (x, y) = geom.node_xy(i, j);
                values[[i, j]] = intercept + gx * x + gy * y;
            }
        }
        Surface::new(geom, values).unwrap()
    }

    #[test]
    fn smooth_leaves_constant_field_unchanged() {
        let s = Surface::constant(geom(5, 10.0), 7.0);
        let out = s.smooth(1);
        for v in out.values().iter() {
            assert_relative_eq!(*v, 7.0);
        }
    }

    #[test]
    fn smooth_reduces_a_spike() {
        let mut v = ndarray::Array2::from_elem((3, 3), 0.0);
        v[[1, 1]] = 9.0; // central spike
        let s = Surface::new(geom(3, 10.0), v).unwrap();
        let out = s.smooth(1);
        // Centre averages the 3×3 window (one 9, eight 0s) → 1.0; spike flattened.
        assert_relative_eq!(out.values()[[1, 1]], 1.0);
        assert!(out.values()[[1, 1]] < 9.0);
    }

    #[test]
    fn smooth_preserves_nan_mask() {
        let mut v = ndarray::Array2::from_elem((3, 3), 1.0);
        v[[0, 0]] = f64::NAN;
        let out = Surface::new(geom(3, 10.0), v).unwrap().smooth(1);
        assert!(out.values()[[0, 0]].is_nan()); // undefined centre stays undefined
        assert_relative_eq!(out.values()[[2, 2]], 1.0);
        assert!(out.attr_names().is_empty());
        assert!(out
            .history()
            .last()
            .unwrap()
            .contains("surface.smooth(radius=1)"));
    }

    #[test]
    fn smooth_preserves_mask_even_with_opposing_infinities() {
        let mut values = Array2::from_elem((3, 3), 1.0);
        values[[0, 0]] = f64::NAN;
        values[[1, 0]] = f64::INFINITY;
        values[[0, 1]] = f64::NEG_INFINITY;
        let source = Surface::new(geom(3, 1.0), values).unwrap();
        let out = source.smooth(1);
        for (&before, &after) in source.values().iter().zip(out.values()) {
            assert_eq!(before.is_nan(), after.is_nan());
        }
    }

    #[test]
    fn dip_matches_world_plane_under_rotation_and_yflip() {
        let geom = GridGeometry {
            xinc: 2.0,
            yinc: 3.0,
            ncol: 4,
            nrow: 5,
            rotation_deg: 37.0,
            yflip: true,
            ..geom(4, 1.0)
        };
        let (gx, gy) = (0.2, -0.1);
        let s = plane(geom, gx, gy, 100.0);
        let angle = s.dip_angle();
        let azimuth = s.dip_azimuth();
        let expected_angle = gx.hypot(gy).atan().to_degrees();
        let expected_azimuth = (-gx).atan2(-gy).to_degrees().rem_euclid(360.0);
        for &v in angle.values() {
            assert_relative_eq!(v, expected_angle, epsilon = 1e-10);
        }
        for &v in azimuth.values() {
            assert_relative_eq!(v, expected_azimuth, epsilon = 1e-10);
        }
        assert!(angle.attr_names().is_empty());
        assert!(azimuth.attr_names().is_empty());
        assert!(angle
            .history()
            .last()
            .unwrap()
            .contains("surface.dip_angle()"));
        assert!(azimuth
            .history()
            .last()
            .unwrap()
            .contains("surface.dip_azimuth()"));
    }

    #[test]
    fn dip_cardinal_directions_flat_and_nan_fallback() {
        for (gx, gy, expected) in [
            (0.0, -1.0, 0.0),
            (-1.0, 0.0, 90.0),
            (0.0, 1.0, 180.0),
            (1.0, 0.0, 270.0),
        ] {
            let azimuth = plane(geom(3, 1.0), gx, gy, 10.0).dip_azimuth();
            assert_relative_eq!(azimuth.values()[[1, 1]], expected, epsilon = 1e-12);
        }

        let flat = Surface::constant(geom(3, 1.0), 7.0);
        assert_eq!(flat.dip_angle().values()[[1, 1]], 0.0);
        assert!(flat.dip_azimuth().values()[[1, 1]].is_nan());

        let geometry = geom(3, 1.0);
        let mut values = plane(geometry.clone(), 1.0, 1.0, 0.0).values().clone();
        values[[1, 1]] = f64::NAN;
        let with_hole = Surface::new(geometry, values).unwrap();
        let angle = with_hole.dip_angle();
        assert!(angle.values()[[1, 1]].is_nan()); // source hole
        assert!(angle.values()[[1, 0]].is_nan()); // J derivative unavailable

        let geometry = geom(5, 1.0);
        let mut values = plane(geometry.clone(), 1.0, 1.0, 0.0).values().clone();
        values[[2, 2]] = f64::NAN;
        let beside_hole = Surface::new(geometry, values).unwrap().dip_angle();
        // The missing +I neighbour falls back to the defined -I neighbour.
        assert_relative_eq!(
            beside_hole.values()[[1, 2]],
            2.0_f64.sqrt().atan().to_degrees()
        );
    }

    #[test]
    fn extrapolate_constant_holes_all_methods_and_preserves_defined_bits() {
        let mut values = Array2::from_elem((5, 5), 7.0);
        values[[2, 2]] = f64::NAN;
        values[[1, 3]] = f64::NAN;
        values[[0, 4]] = f64::INFINITY;
        values[[4, 0]] = f64::NEG_INFINITY;
        let mut source = Surface::new(geom(5, 10.0), values.clone()).unwrap();
        source
            .set_attr("unused", Array2::from_elem((5, 5), 1.0))
            .unwrap();

        for method in [
            GridMethod::Nearest,
            GridMethod::InverseDistance,
            GridMethod::MinimumCurvature,
        ] {
            let out = source.extrapolate(method).unwrap();
            assert_relative_eq!(out.values()[[2, 2]], 7.0, epsilon = 1e-8);
            assert_relative_eq!(out.values()[[1, 3]], 7.0, epsilon = 1e-8);
            for ((i, j), &before) in values.indexed_iter() {
                if !before.is_nan() {
                    assert_eq!(out.values()[[i, j]].to_bits(), before.to_bits());
                }
            }
            assert!(out.attr_names().is_empty());
            assert!(out
                .history()
                .last()
                .unwrap()
                .contains("surface.extrapolate"));
        }
    }

    #[test]
    fn extrapolate_requires_finite_controls_but_nan_free_is_identity() {
        let all_nan = Surface::constant(geom(3, 1.0), f64::NAN);
        let err = all_nan.extrapolate(GridMethod::Nearest).err().unwrap();
        assert!(err
            .to_string()
            .contains("requires at least one finite source node"));

        let mut all_infinite = Surface::constant(geom(3, 1.0), f64::INFINITY);
        all_infinite
            .set_attr("unused", Array2::from_elem((3, 3), 1.0))
            .unwrap();
        let identity = all_infinite.extrapolate(GridMethod::Nearest).unwrap();
        for (&before, &after) in all_infinite.values().iter().zip(identity.values()) {
            assert_eq!(before.to_bits(), after.to_bits());
        }
        assert!(identity.attr_names().is_empty());
    }

    #[test]
    fn edge_hull_area() {
        // 3×3 fully-defined grid, nodes at 0/10/20 → hull is the 20×20 square.
        let s = Surface::constant(geom(3, 10.0), 1.0);
        let poly = s.edge().unwrap();
        assert_relative_eq!(poly.area(), 400.0, epsilon = 1e-9);
    }

    #[test]
    fn edge_none_when_too_few_defined() {
        let mut v = ndarray::Array2::from_elem((3, 3), f64::NAN);
        v[[0, 0]] = 1.0;
        v[[1, 1]] = 1.0; // only two defined nodes
        let s = Surface::new(geom(3, 10.0), v).unwrap();
        assert!(s.edge().is_none());
    }
}
