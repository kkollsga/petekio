//! `Surface` — a regular gridded surface (the workhorse): a primary value layer
//! plus named attribute layers on the same `GridGeometry`. `NaN` = undefined.
//!
//! This module covers construction, IO, and access. Math/sampling/statistics
//! land in later phases.

use crate::foundation::{GeoError, GridGeometry, HasHistory, OperationHistory, Result};
use crate::io::SurfaceData;
use indexmap::IndexMap;
use ndarray::Array2;
use std::path::Path;

/// A rotated regular grid (IRAP/RMS model) holding a primary value layer
/// (`values`, e.g. depth) plus named attribute layers (thickness, seismic, …)
/// on the same geometry. Undefined nodes are `NaN`.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct Surface {
    /// The areal lattice. Public; `values`/`attributes` are private.
    pub geom: GridGeometry,
    values: Array2<f64>,
    attributes: IndexMap<String, Array2<f64>>,
    #[serde(default)]
    history: OperationHistory,
}

impl Surface {
    /// Build a surface from a geometry and a primary value grid. The grid must
    /// be shape `(ncol, nrow)` or `GeometryMismatch` is returned.
    pub fn new(geom: GridGeometry, values: Array2<f64>) -> Result<Surface> {
        check_shape(&geom, &values, "Surface::new")?;
        Ok(Surface {
            geom,
            values,
            attributes: IndexMap::new(),
            history: OperationHistory::from_entry("surface.new"),
        })
    }

    pub(crate) fn from_surface_data(data: SurfaceData) -> Surface {
        let (geom, values, attributes) = data.into_parts();
        Surface {
            geom,
            values,
            attributes,
            history: OperationHistory::from_entry("surface.import"),
        }
    }

    /// Build a surface from a geometry + values without shape validation, for
    /// internal callers (operations) that already guarantee the shape. No
    /// attributes are carried over.
    pub(crate) fn from_values_unchecked(geom: GridGeometry, values: Array2<f64>) -> Surface {
        Surface {
            geom,
            values,
            attributes: IndexMap::new(),
            history: OperationHistory::new(),
        }
    }

    /// A surface whose every node holds `value`.
    pub fn constant(geom: GridGeometry, value: f64) -> Surface {
        let values = Array2::from_elem((geom.ncol, geom.nrow), value);
        Surface {
            geom,
            values,
            attributes: IndexMap::new(),
            history: OperationHistory::from_entry(format!("surface.constant(value={value})")),
        }
    }

    /// Load an IRAP-classic (ROXAR ASCII) surface — the first supported format.
    pub fn load_irap_classic(path: impl AsRef<Path>) -> Result<Surface> {
        let data = crate::io::irap::load_irap_classic(path.as_ref())?;
        let mut out = Surface::from_surface_data(data);
        out.history = OperationHistory::from_entry(format!(
            "surface.load_irap_classic(path={})",
            path.as_ref().display()
        ));
        Ok(out)
    }

    /// Load a CPS-3 regular grid (`.CPS3grid`) — `FS*` header + row-major z, the
    /// `1.0E+30`-family null → `NaN`, north-to-south node ordering (see
    /// [`crate::io::cps3`]).
    pub fn load_cps3_grid(path: impl AsRef<Path>) -> Result<Surface> {
        let data = crate::io::cps3::load_cps3_grid(path.as_ref())?;
        let mut out = Surface::from_surface_data(data);
        out.history = OperationHistory::from_entry(format!(
            "surface.load_cps3_grid(path={})",
            path.as_ref().display()
        ));
        Ok(out)
    }

    /// Write this surface's primary layer as IRAP-classic ASCII.
    pub fn save_irap_classic(&self, path: impl AsRef<Path>) -> Result<()> {
        crate::io::irap::save_irap_classic(path.as_ref(), &self.geom, &self.values)
    }

    /// The primary value grid, shape `(ncol, nrow)`. `NaN` = undefined.
    pub fn values(&self) -> &Array2<f64> {
        &self.values
    }

    /// A named attribute grid, if present.
    pub fn attr(&self, name: &str) -> Option<&Array2<f64>> {
        self.attributes.get(name)
    }

    /// Set (or replace) a named attribute grid. Must match the surface
    /// geometry or `GeometryMismatch` is returned.
    pub fn set_attr(&mut self, name: &str, values: Array2<f64>) -> Result<()> {
        check_shape(&self.geom, &values, "Surface::set_attr")?;
        self.attributes.insert(name.to_string(), values);
        self.record_history(format!("surface.set_attr(name={name})"));
        Ok(())
    }

    /// The names of all attribute layers, in insertion order.
    pub fn attr_names(&self) -> Vec<&str> {
        self.attributes.keys().map(String::as_str).collect()
    }

    /// Promote an attribute layer to a standalone `Surface` (its primary
    /// values), so surface operations can run on it.
    pub fn as_attr_surface(&self, name: &str) -> Option<Surface> {
        self.attributes.get(name).map(|a| Surface {
            geom: self.geom.clone(),
            values: a.clone(),
            attributes: IndexMap::new(),
            history: self.history_with(format!("surface.as_attr_surface(name={name})")),
        })
    }

    /// Human-readable operation history for this surface.
    pub fn history(&self) -> &[String] {
        self.history.entries()
    }

    pub(crate) fn history_with(&self, entry: impl Into<String>) -> OperationHistory {
        self.history.with_entry(entry)
    }

    pub(crate) fn record_history(&mut self, entry: impl Into<String>) {
        self.history.push(entry.into());
    }

    pub(crate) fn set_history(&mut self, history: impl Into<OperationHistory>) {
        self.history = history.into();
    }

    /// Bilinear sample of the primary layer at world `(x, y)`. Single-homed on
    /// the shared resample kernel (`petektools::resample`, Bilinear) via a 1×1
    /// target lattice — one home for the bilinear math.
    ///
    /// `None` if the point is outside the grid. **NaN-corner policy (kernel):**
    /// if the *nearest* of the four surrounding source corners is undefined the
    /// result is `None`; otherwise it is the weighted mean over the **finite**
    /// corners with the weights renormalized (a `NaN` far corner is dropped, not
    /// treated as zero). This CHANGED at the centralization: petekIO previously
    /// hard-holed on ANY undefined corner. See the crate CHANGELOG.
    ///
    /// A rotated/`yflip`ed source is honoured exactly here — a point query is a
    /// single world→index map, valid under rotation even though grid
    /// [`resample`](Self::resample) gates it.
    pub fn sample(&self, x: f64, y: f64) -> Option<f64> {
        let src = self.geom.to_lattice();
        // 1×1 target lattice at the query point; spacing is irrelevant (single
        // node), rotation 0.
        let target = petektools::Lattice::regular(x, y, 1.0, 1.0, 1, 1);
        let out = petektools::resample(
            &self.values,
            &src,
            &target,
            petektools::ResampleMethod::Bilinear,
        )
        .ok()?;
        let v = out[[0, 0]];
        v.is_finite().then_some(v)
    }

    /// Resample the primary layer onto a target geometry (bilinear). Single-homed
    /// on the shared resample kernel (`petektools::resample`, Bilinear) — the one
    /// resampler. Target nodes outside this surface become `NaN`; the kernel's
    /// NaN-corner policy applies (nearest corner `NaN` → `NaN`, else renormalized
    /// over the finite corners — see [`sample`](Self::sample) and the CHANGELOG).
    ///
    /// **Rotation guard.** The shared kernel is **axis-aligned-only**. If either
    /// this surface's or the target's geometry is rotated (`rotation_deg != 0`),
    /// this returns [`GeoError::Unsupported`] rather than a silent wrong answer,
    /// until the kernel gains rotation support (suite task_suite_grid_rotation).
    /// `yflip` is fully supported.
    pub fn resample(&self, target: &GridGeometry) -> Result<Surface> {
        if !self.geom.is_axis_aligned() || !target.is_axis_aligned() {
            return Err(GeoError::Unsupported(format!(
                "resample: rotated grid geometry is not supported by the shared \
                 axis-aligned resample kernel (source rotation_deg={}, target \
                 rotation_deg={}); axis-aligned + yflip only",
                self.geom.rotation_deg, target.rotation_deg
            )));
        }
        let values = petektools::resample(
            &self.values,
            &self.geom.to_lattice(),
            &target.to_lattice(),
            petektools::ResampleMethod::Bilinear,
        )?;
        let mut out = Surface {
            geom: target.clone(),
            values,
            attributes: IndexMap::new(),
            history: OperationHistory::new(),
        };
        out.set_history(self.history_with(format!(
            "surface.resample(ncol={}, nrow={})",
            target.ncol, target.nrow
        )));
        Ok(out)
    }
}

impl HasHistory for Surface {
    fn operation_history(&self) -> &OperationHistory {
        &self.history
    }

    fn operation_history_mut(&mut self) -> &mut OperationHistory {
        &mut self.history
    }
}

fn check_shape(geom: &GridGeometry, values: &Array2<f64>, ctx: &str) -> Result<()> {
    if values.dim() != (geom.ncol, geom.nrow) {
        return Err(GeoError::GeometryMismatch(format!(
            "{ctx}: values shape {:?} != grid (ncol={}, nrow={})",
            values.dim(),
            geom.ncol,
            geom.nrow
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    /// A 2×2 axis-aligned surface with corner values 0/10/20/30 (i along x).
    fn ramp() -> Surface {
        let mut v = Array2::zeros((2, 2));
        v[[0, 0]] = 0.0;
        v[[1, 0]] = 10.0;
        v[[0, 1]] = 20.0;
        v[[1, 1]] = 30.0;
        Surface::new(geom(), v).unwrap()
    }

    #[test]
    fn bilinear_sample_hand_calc() {
        let s = ramp();
        assert_relative_eq!(s.sample(5.0, 5.0).unwrap(), 15.0); // centre = mean
        assert_relative_eq!(s.sample(2.0, 0.0).unwrap(), 2.0); // along bottom edge
        assert_relative_eq!(s.sample(0.0, 0.0).unwrap(), 0.0); // origin node
        assert_eq!(s.sample(-1.0, 0.0), None); // outside
        assert_eq!(s.sample(100.0, 100.0), None); // outside
    }

    /// NaN-corner policy (kernel, post-centralization). A 2×2 with an undefined
    /// [1,1] corner.
    #[test]
    fn sample_nan_corner_policy() {
        let mut v = Array2::zeros((2, 2));
        v[[0, 0]] = 0.0;
        v[[1, 0]] = 10.0;
        v[[0, 1]] = 20.0;
        v[[1, 1]] = f64::NAN;
        let s = Surface::new(geom(), v).unwrap();
        // (a) NEAREST corner is the hole: (5,5) → fi=fj=0.5 → round → corner
        //     (1,1) = NaN ⇒ None (unchanged from the old hard-hole behaviour
        //     for this point).
        assert_eq!(s.sample(5.0, 5.0), None);
        // (b) BEHAVIOUR CHANGE: nearest corner FINITE but a far corner is the
        //     hole. (3,3) → fi=fj=0.3, nearest (0,0)=0 finite; corner (1,1) is
        //     the hole. Old petekIO hard-holed → None. The kernel renormalizes
        //     over the finite corners → Some. Hand calc:
        //       (0·.49 + 10·.21 + 20·.21) / (.49 + .21 + .21) = 6.3 / 0.91.
        let got = s
            .sample(3.0, 3.0)
            .expect("finite corners must fill the fringe");
        assert_relative_eq!(got, 6.3 / 0.91, epsilon = 1e-12);
    }

    #[test]
    fn resample_interpolates_and_copies_geometry() {
        let s = ramp();
        let target = GridGeometry {
            xori: 0.0,
            yori: 0.0,
            xinc: 5.0,
            yinc: 5.0,
            ncol: 2,
            nrow: 2,
            rotation_deg: 0.0,
            yflip: false,
        };
        let r = s.resample(&target).unwrap();
        assert_eq!(r.geom, target);
        assert_relative_eq!(r.values()[[0, 0]], 0.0);
        assert_relative_eq!(r.values()[[1, 1]], 15.0); // (5,5) → centre
    }

    /// R1 world-frame variant: resample across a NON-trivial world frame —
    /// source and target differ in origin AND spacing (and are `yflip`ed) — must
    /// return the field sampled at each target node's WORLD position, proving the
    /// georeference is honoured through the kernel seam (not an index-for-index
    /// copy). Bilinear is exact on an affine field.
    #[test]
    fn resample_honours_world_frame() {
        // Affine (planar) field in world coordinates.
        let f = |x: f64, y: f64| 3.0 + 0.5 * (x - 1000.0) - 0.25 * (y - 2000.0);
        let src_geom = GridGeometry {
            xori: 1000.0,
            yori: 2000.0,
            xinc: 10.0,
            yinc: 10.0,
            ncol: 5,
            nrow: 5,
            rotation_deg: 0.0,
            yflip: true, // exercise the honoured flip
        };
        let mut sv = Array2::zeros((src_geom.ncol, src_geom.nrow));
        for j in 0..src_geom.nrow {
            for i in 0..src_geom.ncol {
                let (x, y) = src_geom.node_xy(i, j);
                sv[[i, j]] = f(x, y);
            }
        }
        let s = Surface::new(src_geom.clone(), sv).unwrap();
        // Target: offset origin, different spacing, same flip — inside the source.
        let target = GridGeometry {
            xori: 1005.0,
            yori: 1995.0,
            xinc: 8.0,
            yinc: 8.0,
            ncol: 3,
            nrow: 3,
            rotation_deg: 0.0,
            yflip: true,
        };
        let r = s.resample(&target).unwrap();
        for j in 0..target.nrow {
            for i in 0..target.ncol {
                let (x, y) = target.node_xy(i, j);
                let v = r.values()[[i, j]];
                assert!(v.is_finite(), "node ({i},{j}) at world ({x},{y}) is NaN");
                assert_relative_eq!(v, f(x, y), epsilon = 1e-9);
            }
        }
    }

    /// Rotation guard: a rotated source OR target is a typed `Unsupported`
    /// error, never a silent wrong answer (the kernel is axis-aligned-only).
    #[test]
    fn resample_rotated_is_unsupported() {
        let s = ramp();
        let mut rotated = geom();
        rotated.rotation_deg = 30.0;
        // rotated TARGET
        assert!(matches!(
            s.resample(&rotated),
            Err(GeoError::Unsupported(_))
        ));
        // rotated SOURCE
        let s_rot = Surface::new(rotated.clone(), Array2::zeros((2, 2))).unwrap();
        assert!(matches!(
            s_rot.resample(&geom()),
            Err(GeoError::Unsupported(_))
        ));
    }

    fn geom() -> GridGeometry {
        GridGeometry {
            xori: 0.0,
            yori: 0.0,
            xinc: 10.0,
            yinc: 10.0,
            ncol: 2,
            nrow: 2,
            rotation_deg: 0.0,
            yflip: false,
        }
    }

    #[test]
    fn new_rejects_wrong_shape() {
        let bad = Array2::from_elem((3, 3), 1.0);
        assert!(Surface::new(geom(), bad).is_err());
    }

    #[test]
    fn attributes_set_get_promote() {
        let mut s = Surface::constant(geom(), 1.0);
        s.set_attr("thickness", Array2::from_elem((2, 2), 5.0))
            .unwrap();
        assert_eq!(s.attr_names(), vec!["thickness"]);
        assert_eq!(s.attr("thickness").unwrap()[[0, 0]], 5.0);
        assert!(s.attr("missing").is_none());
        let promoted = s.as_attr_surface("thickness").unwrap();
        assert_eq!(promoted.values()[[1, 1]], 5.0);
        // wrong-shape attr rejected
        assert!(s.set_attr("bad", Array2::from_elem((1, 1), 0.0)).is_err());
    }
}
