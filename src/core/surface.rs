//! `Surface` — a regular gridded surface (the workhorse): a primary value layer
//! plus named attribute layers on the same `GridGeometry`. `NaN` = undefined.
//!
//! This module covers construction, IO, and access. Math/sampling/statistics
//! land in later phases.

use crate::core::attribute::{
    check_metadata_name, validate_attribute_values, AttributeLane, AttributeMetadata,
};
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
    #[serde(default)]
    primary_metadata: Option<AttributeMetadata>,
    attributes: IndexMap<String, AttributeLane<Array2<f64>>>,
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
            primary_metadata: None,
            attributes: IndexMap::new(),
            history: OperationHistory::from_entry("surface.new"),
        })
    }

    pub(crate) fn from_surface_data(data: SurfaceData) -> Surface {
        let (geom, values, attributes) = data.into_parts();
        let mut out = Surface {
            geom,
            values,
            primary_metadata: None,
            attributes: IndexMap::new(),
            history: OperationHistory::from_entry("surface.import"),
        };
        for (name, values) in attributes {
            // Reader-produced lanes predate durable metadata and therefore use
            // the same honest legacy defaults as public values-only authoring.
            out.set_attr(&name, values)
                .expect("SurfaceData validated every attribute shape");
        }
        out
    }

    /// Build a surface from a geometry + values without shape validation, for
    /// internal callers (operations) that already guarantee the shape. No
    /// attributes are carried over.
    pub(crate) fn from_values_unchecked(geom: GridGeometry, values: Array2<f64>) -> Surface {
        Surface {
            geom,
            values,
            primary_metadata: None,
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
            primary_metadata: None,
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
        self.attributes.get(name).map(|lane| &lane.values)
    }

    /// Durable metadata for a named attribute lane.
    pub fn attr_metadata(&self, name: &str) -> Option<&AttributeMetadata> {
        self.attributes.get(name).map(|lane| &lane.metadata)
    }

    /// Metadata carried by the primary lane after attribute promotion.
    pub fn primary_metadata(&self) -> Option<&AttributeMetadata> {
        self.primary_metadata.as_ref()
    }

    pub(crate) fn set_primary_metadata(&mut self, metadata: Option<AttributeMetadata>) {
        self.primary_metadata = metadata;
    }

    /// Set (or replace) a named attribute grid. Must match the surface
    /// geometry or `GeometryMismatch` is returned.
    pub fn set_attr(&mut self, name: &str, values: Array2<f64>) -> Result<()> {
        check_shape(&self.geom, &values, "Surface::set_attr")?;
        if let Some(existing) = self.attributes.get_mut(name) {
            validate_attribute_values(&existing.metadata, values.iter())?;
            existing.values = values;
        } else {
            let metadata = AttributeMetadata::continuous(name)?;
            self.attributes
                .insert(name.to_string(), AttributeLane::new(metadata, values)?);
        }
        self.record_history(format!("surface.set_attr(name={name})"));
        Ok(())
    }

    /// Set (or replace) values and explicitly override their durable metadata.
    pub fn set_attr_with_metadata(
        &mut self,
        name: &str,
        values: Array2<f64>,
        metadata: AttributeMetadata,
    ) -> Result<()> {
        check_shape(&self.geom, &values, "Surface::set_attr_with_metadata")?;
        check_metadata_name(name, &metadata)?;
        validate_attribute_values(&metadata, values.iter())?;
        self.attributes
            .insert(name.to_string(), AttributeLane::new(metadata, values)?);
        self.record_history(format!("surface.set_attr_with_metadata(name={name})"));
        Ok(())
    }

    /// Explicitly replace metadata for an existing attribute without touching values.
    pub fn set_attr_metadata(&mut self, name: &str, metadata: AttributeMetadata) -> Result<()> {
        check_metadata_name(name, &metadata)?;
        let lane = self
            .attributes
            .get_mut(name)
            .ok_or_else(|| GeoError::NotFound(format!("no attribute layer '{name}'")))?;
        validate_attribute_values(&metadata, lane.values.iter())?;
        lane.metadata = metadata;
        self.record_history(format!("surface.set_attr_metadata(name={name})"));
        Ok(())
    }

    /// The names of all attribute layers, in insertion order.
    pub fn attr_names(&self) -> Vec<&str> {
        self.attributes.keys().map(String::as_str).collect()
    }

    /// Promote an attribute layer to a standalone `Surface` (its primary
    /// values), so surface operations can run on it.
    pub fn as_attr_surface(&self, name: &str) -> Option<Surface> {
        self.attributes.get(name).map(|lane| Surface {
            geom: self.geom.clone(),
            values: lane.values.clone(),
            primary_metadata: Some(lane.metadata.clone()),
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
    /// A rotated/`yflip`ed source is honoured exactly here through the same
    /// world→intrinsic transform used by [`resample`](Self::resample).
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
    /// Source and target may carry independent intrinsic rotation and `yflip`;
    /// the shared kernel maps target nodes through world coordinates into the
    /// source's intrinsic index frame before interpolation.
    pub fn resample(&self, target: &GridGeometry) -> Result<Surface> {
        let method = match self.primary_metadata.as_ref().map(|meta| meta.kind) {
            Some(crate::AttributeKind::Categorical) => petektools::ResampleMethod::Nearest,
            _ => petektools::ResampleMethod::Bilinear,
        };
        let values = petektools::resample(
            &self.values,
            &self.geom.to_lattice(),
            &target.to_lattice(),
            method,
        )?;
        let mut out = Surface {
            geom: target.clone(),
            values,
            primary_metadata: self.primary_metadata.clone(),
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

#[derive(serde::Serialize, serde::Deserialize)]
struct SurfaceV1 {
    geom: GridGeometry,
    values: Array2<f64>,
    attributes: IndexMap<String, Array2<f64>>,
    #[serde(default)]
    history: OperationHistory,
}

impl Surface {
    pub(crate) fn from_v1_payload(bytes: &[u8]) -> Result<Self> {
        let old: SurfaceV1 = crate::io::serial::from_bytes(bytes)?;
        let mut out = Surface::new(old.geom, old.values)?;
        for (name, values) in old.attributes {
            out.set_attr(&name, values)?;
        }
        out.history = old.history;
        Ok(out)
    }

    pub(crate) fn validate_metadata(&self) -> Result<()> {
        if let Some(metadata) = &self.primary_metadata {
            metadata.validate()?;
            validate_attribute_values(metadata, self.values.iter())?;
        }
        for (name, lane) in &self.attributes {
            check_metadata_name(name, &lane.metadata)?;
            validate_attribute_values(&lane.metadata, lane.values.iter())?;
        }
        Ok(())
    }

    pub(crate) fn migrate_persisted_metadata_text(&mut self) {
        if let Some(metadata) = &mut self.primary_metadata {
            metadata.migrate_persisted_text();
        }
        for lane in self.attributes.values_mut() {
            lane.metadata.migrate_persisted_text();
        }
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

    /// Rotated source and target share one exact world frame with the
    /// petekTools lattice seam. Bilinear interpolation reproduces an affine
    /// world field exactly.
    #[test]
    fn resample_rotated_affine_world_field_is_exact() {
        let field = |x: f64, y: f64| 3.0 + 0.5 * (x - 431_000.0) - 0.25 * (y - 6_521_000.0);
        let source = GridGeometry {
            xori: 431_000.0,
            yori: 6_521_000.0,
            xinc: 10.0,
            yinc: 12.0,
            ncol: 5,
            nrow: 5,
            rotation_deg: 30.0,
            yflip: true,
        };
        let values = Array2::from_shape_fn((source.ncol, source.nrow), |(i, j)| {
            let (x, y) = source.node_xy(i, j);
            field(x, y)
        });
        let surface = Surface::new(source.clone(), values).unwrap();
        let target = GridGeometry {
            xinc: 5.0,
            yinc: 6.0,
            ncol: 9,
            nrow: 9,
            ..source
        };
        let out = surface.resample(&target).unwrap();
        for j in 0..target.nrow {
            for i in 0..target.ncol {
                let (x, y) = target.node_xy(i, j);
                assert_relative_eq!(out.values()[[i, j]], field(x, y), epsilon = 1e-8);
                assert_relative_eq!(surface.sample(x, y).unwrap(), field(x, y), epsilon = 1e-8);
            }
        }
    }

    #[test]
    fn promoted_categorical_rotated_resample_uses_nearest() {
        let source = GridGeometry {
            xori: 431_000.0,
            yori: 6_521_000.0,
            xinc: 10.0,
            yinc: 12.0,
            ncol: 3,
            nrow: 3,
            rotation_deg: 30.0,
            yflip: true,
        };
        let mut surface = Surface::constant(source.clone(), -1800.0);
        let facies = Array2::from_shape_fn((3, 3), |(i, j)| (i + 10 * j) as f64);
        surface
            .set_attr_with_metadata(
                "facies",
                facies.clone(),
                AttributeMetadata::new(
                    "facies",
                    "Facies",
                    crate::AttributeKind::Categorical,
                    None,
                    None,
                )
                .unwrap(),
            )
            .unwrap();
        let promoted = surface.as_attr_surface("facies").unwrap();
        let target = GridGeometry {
            xinc: 4.0,
            yinc: 4.8,
            ncol: 6,
            nrow: 6,
            ..source
        };
        let out = promoted.resample(&target).unwrap();
        assert_eq!(out.primary_metadata(), promoted.primary_metadata());
        for j in 0..target.nrow {
            for i in 0..target.ncol {
                let expected = facies[[
                    (i as f64 * 0.4).round() as usize,
                    (j as f64 * 0.4).round() as usize,
                ]];
                assert_eq!(out.values()[[i, j]], expected);
            }
        }
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

    #[test]
    fn metadata_survives_replacement_promotion_and_v2_round_trip() {
        let mut s = Surface::constant(geom(), 1.0);
        let metadata = AttributeMetadata::new(
            "porosity",
            "Porosity",
            crate::AttributeKind::Continuous,
            Some("v/v".into()),
            None,
        )
        .unwrap();
        s.set_attr_with_metadata("porosity", Array2::from_elem((2, 2), 0.2), metadata.clone())
            .unwrap();
        s.set_attr("porosity", Array2::from_elem((2, 2), 0.25))
            .unwrap();
        assert_eq!(s.attr_metadata("porosity"), Some(&metadata));
        assert_eq!(
            s.as_attr_surface("porosity").unwrap().primary_metadata(),
            Some(&metadata)
        );

        let bytes = crate::io::serial::to_bytes(&s).unwrap();
        let back: Surface = crate::io::serial::from_bytes(&bytes).unwrap();
        assert_eq!(back.attr_metadata("porosity"), Some(&metadata));
    }

    #[test]
    fn categorical_values_must_be_integral_on_authoring_and_replacement() {
        let mut s = Surface::constant(geom(), 1.0);
        let categorical = AttributeMetadata::new(
            "facies",
            "Facies",
            crate::AttributeKind::Categorical,
            None,
            None,
        )
        .unwrap();
        let valid = ndarray::array![[1.0, f64::NAN], [2.0, 3.0]];
        s.set_attr_with_metadata("facies", valid.clone(), categorical.clone())
            .unwrap();

        assert!(s
            .set_attr("facies", Array2::from_elem((2, 2), 1.5))
            .is_err());
        let preserved = s.attr("facies").unwrap();
        assert_eq!(preserved[[0, 0]], 1.0);
        assert!(preserved[[0, 1]].is_nan());
        assert_eq!(preserved[[1, 0]], 2.0);
        assert_eq!(preserved[[1, 1]], 3.0);
        assert!(s
            .set_attr_with_metadata(
                "fractional",
                Array2::from_elem((2, 2), 1.5),
                AttributeMetadata::new(
                    "fractional",
                    "Fractional",
                    crate::AttributeKind::Categorical,
                    None,
                    None,
                )
                .unwrap(),
            )
            .is_err());

        s.set_attr("continuous", Array2::from_elem((2, 2), 1.5))
            .unwrap();
        assert!(s
            .set_attr_metadata(
                "continuous",
                AttributeMetadata::new(
                    "continuous",
                    "Continuous",
                    crate::AttributeKind::Categorical,
                    None,
                    None,
                )
                .unwrap(),
            )
            .is_err());
    }

    #[test]
    fn positional_v1_payload_migrates_with_honest_defaults() {
        let mut attributes = IndexMap::new();
        attributes.insert("legacy".into(), Array2::from_elem((2, 2), 3.0));
        let old = SurfaceV1 {
            geom: geom(),
            values: Array2::from_elem((2, 2), 1.0),
            attributes,
            history: OperationHistory::from_entry("v1.fixture"),
        };
        let bytes = crate::io::serial::to_bytes(&old).unwrap();
        let migrated = Surface::from_v1_payload(&bytes).unwrap();
        assert_eq!(
            migrated.attr_metadata("legacy"),
            Some(&AttributeMetadata::continuous("legacy").unwrap())
        );
        assert_eq!(migrated.history(), &["v1.fixture"]);
    }

    #[test]
    fn promoted_categorical_primary_resamples_with_nearest() {
        let mut s = Surface::constant(geom(), 0.0);
        s.set_attr_with_metadata(
            "facies",
            ndarray::array![[1.0, 1.0], [2.0, 2.0]],
            AttributeMetadata::new(
                "facies",
                "Facies",
                crate::AttributeKind::Categorical,
                None,
                None,
            )
            .unwrap(),
        )
        .unwrap();
        let promoted = s.as_attr_surface("facies").unwrap();
        let target = GridGeometry {
            xori: 5.0,
            yori: 0.0,
            xinc: 10.0,
            yinc: 10.0,
            ncol: 1,
            nrow: 2,
            rotation_deg: 0.0,
            yflip: false,
        };
        let down = promoted.resample(&target).unwrap();
        assert!(down.values().iter().all(|value| value.fract() == 0.0));
        assert_eq!(
            down.primary_metadata().unwrap().kind,
            crate::AttributeKind::Categorical
        );
    }
}
