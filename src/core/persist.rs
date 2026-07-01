//! Per-element persistence: save/load a single `Surface` / `Well` / `PointSet` /
//! `PolygonSet` as a standalone one-section `.pproj`. The whole-project
//! orchestration lives in `manager`; this is the "each element individually"
//! surface (mirrors `Surface::save_irap_classic`). Bulk arrays are bincode'd
//! (NaN-safe) then `zstd`'d by the container.

use crate::core::{PointSet, PolygonSet, Surface, Well};
use crate::foundation::{GeoError, Result};
use crate::io::{
    container::{self, Section},
    serial,
};
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::path::Path;

use serial::DATA_VERSION;

/// One-section persistence mapping for a domain element: its `.pproj` section
/// `kind`, its section name, and the (de)serialization. Defined **once per
/// type** so both single-element save (here) and whole-project save (`manager`)
/// agree on the kind strings — add a new element type by implementing this, and
/// nothing else needs to learn its `kind` (open/closed).
pub(crate) trait Persistable: Serialize + DeserializeOwned + Sized {
    /// Stable on-disk section-`kind` tag.
    const KIND: &'static str;
    /// This element's section name (its identity within a project).
    fn element_name(&self) -> String;
    /// Frame this element as a container [`Section`] — no tags (a project-level
    /// concern the caller sets), version pinned to [`DATA_VERSION`].
    fn to_section(&self) -> Result<Section> {
        Ok(Section {
            kind: Self::KIND.to_string(),
            name: self.element_name(),
            tags: Vec::new(),
            version: DATA_VERSION,
            payload: serial::to_bytes(self)?,
        })
    }
    /// Decode this element from a section payload.
    fn from_payload(bytes: &[u8]) -> Result<Self> {
        serial::from_bytes(bytes)
    }
}

impl Persistable for Surface {
    const KIND: &'static str = "surface";
    fn element_name(&self) -> String {
        "surface".to_string()
    }
}
impl Persistable for Well {
    const KIND: &'static str = "well";
    fn element_name(&self) -> String {
        self.id.clone()
    }
}
impl Persistable for PointSet {
    const KIND: &'static str = "points";
    fn element_name(&self) -> String {
        "points".to_string()
    }
}
impl Persistable for PolygonSet {
    const KIND: &'static str = "polygons";
    fn element_name(&self) -> String {
        "polygons".to_string()
    }
}

/// Write one element as a single-section `.pproj`.
fn save_one<T: Persistable>(path: &Path, value: &T) -> Result<()> {
    container::write(
        path,
        &serde_json::json!({}),
        DATA_VERSION,
        &[value.to_section()?],
    )
}

/// Load the first section of `T`'s kind from a `.pproj`.
fn load_one<T: Persistable>(path: &Path) -> Result<T> {
    let mut reader = container::open(path)?;
    let name = reader
        .entries()
        .iter()
        .find(|e| e.kind == T::KIND)
        .ok_or_else(|| {
            GeoError::NotFound(format!("no '{}' section in {}", T::KIND, path.display()))
        })?
        .name
        .clone();
    T::from_payload(&reader.read(&name)?.payload)
}

impl Surface {
    /// Save this surface (geometry + values + attribute layers) to a `.pproj`.
    pub fn save(&self, path: impl AsRef<Path>) -> Result<()> {
        save_one(path.as_ref(), self)
    }
    /// Load a surface previously written with [`save`](Surface::save).
    pub fn load(path: impl AsRef<Path>) -> Result<Surface> {
        load_one(path.as_ref())
    }
}

impl Well {
    /// Save this well (bores, trajectories, logs, tops, CRS) to a `.pproj`.
    pub fn save(&self, path: impl AsRef<Path>) -> Result<()> {
        save_one(path.as_ref(), self)
    }
    /// Load a well previously written with [`save`](Well::save).
    pub fn load(path: impl AsRef<Path>) -> Result<Well> {
        load_one(path.as_ref())
    }
}

impl PointSet {
    /// Save this point set (coordinates + attribute columns) to a `.pproj`.
    pub fn save(&self, path: impl AsRef<Path>) -> Result<()> {
        save_one(path.as_ref(), self)
    }
    /// Load a point set previously written with [`save`](PointSet::save).
    pub fn load(path: impl AsRef<Path>) -> Result<PointSet> {
        load_one(path.as_ref())
    }
}

impl PolygonSet {
    /// Save this polygon set (rings) to a `.pproj`.
    pub fn save(&self, path: impl AsRef<Path>) -> Result<()> {
        save_one(path.as_ref(), self)
    }
    /// Load a polygon set previously written with [`save`](PolygonSet::save).
    pub fn load(path: impl AsRef<Path>) -> Result<PolygonSet> {
        load_one(path.as_ref())
    }
}

impl PointSet {
    /// Export to GeoJSON (`Point` features; attributes → properties). Round-trips
    /// with `load_geojson`. `NaN` → `null`.
    pub fn export_geojson(&self, path: impl AsRef<Path>) -> Result<()> {
        crate::io::vector_write::write_points_geojson(path.as_ref(), &self.coords, &self.attrs)
    }
    /// Export to CSV (`x,y,z` + one column per attribute). Round-trips with
    /// `load_csv`.
    pub fn export_csv(&self, path: impl AsRef<Path>) -> Result<()> {
        crate::io::vector_write::write_points_csv(path.as_ref(), &self.coords, &self.attrs)
    }
}

impl PolygonSet {
    /// Export to GeoJSON (`Polygon` features from the rings). Round-trips with
    /// `load_geojson`.
    pub fn export_geojson(&self, path: impl AsRef<Path>) -> Result<()> {
        crate::io::vector_write::write_polygons_geojson(path.as_ref(), &self.rings())
    }
}

#[cfg(test)]
mod tests {
    use crate::core::log::Log;
    use crate::core::tops::Top;
    use crate::core::trajectory::{Station, TrajectoryInput};
    use crate::core::{PointSet, PolygonSet, Surface, Well};
    use crate::foundation::GridGeometry;
    use indexmap::IndexMap;
    use ndarray::Array2;

    fn tmp(tag: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("pio_persist_{tag}_{}.pproj", std::process::id()))
    }

    fn geom() -> GridGeometry {
        GridGeometry {
            xori: 0.0,
            yori: 0.0,
            xinc: 25.0,
            yinc: 25.0,
            ncol: 3,
            nrow: 2,
            rotation_deg: 0.0,
            yflip: false,
        }
    }

    #[test]
    fn surface_round_trips_incl_nan() {
        let vals = Array2::from_shape_vec((3, 2), vec![1.0, f64::NAN, 2.0, 3.0, 4.0, 5.0]).unwrap();
        let s = Surface::new(geom(), vals).unwrap();
        let p = tmp("surf");
        s.save(&p).unwrap();
        let back = Surface::load(&p).unwrap();
        assert_eq!(back.geom, geom());
        // Sample everywhere; loaded must be bit-identical to original (NaN too).
        for x in [0.0, 25.0, 50.0] {
            for y in [0.0, 25.0] {
                let bits = |o: Option<f64>| o.map(f64::to_bits);
                assert_eq!(bits(s.sample(x, y)), bits(back.sample(x, y)));
            }
        }
        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn well_round_trips_hierarchy() {
        let mut w = Well::new("25/1-1", (1000.0, 2000.0), 30.0);
        w.set_crs("ED50 / UTM zone 31N");
        let st = w.sidetrack_mut("A");
        st.add_trajectory(TrajectoryInput::Stations(vec![
            Station::new(0.0, 0.0, 0.0),
            Station::new(2000.0, 0.0, 0.0),
        ]))
        .unwrap();
        st.add_log(
            Log::new(
                "PHIE",
                "m3/m3",
                vec![1000.0, 1010.0, 1020.0],
                vec![0.2, f64::NAN, 0.25],
            )
            .unwrap(),
        );
        st.add_tops(vec![Top::new("Top A", 1000.0), Top::new("Base A", 1015.0)]);

        let p = tmp("well");
        w.save(&p).unwrap();
        let back = Well::load(&p).unwrap();
        assert_eq!(back.id, "25/1-1");
        assert_eq!(back.head, (1000.0, 2000.0));
        assert_eq!(back.kb, 30.0);
        assert_eq!(back.crs(), Some("ED50 / UTM zone 31N"));
        let b = back.sidetrack("A").expect("bore A round-tripped");
        assert!(b.log("PHIE").is_some());
        let s = b.log("PHIE").unwrap().stats();
        assert_eq!(s.count, 2); // NaN sample skipped, both others counted
        assert!(b.zones().iter().any(|z| z.name == "Top A"));
        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn points_round_trip() {
        let mut attrs = IndexMap::new();
        attrs.insert("poro".to_string(), vec![0.2, f64::NAN, 0.3]);
        let pts = PointSet {
            coords: vec![[0.0, 0.0, 100.0], [1.0, 1.0, 110.0], [2.0, 2.0, 120.0]],
            attrs,
        };
        let p = tmp("pts");
        pts.save(&p).unwrap();
        let back = PointSet::load(&p).unwrap();
        assert_eq!(back.len(), 3);
        let s = back.stats("poro").unwrap();
        assert_eq!(s.count, 2); // NaN skipped
        std::fs::remove_file(&p).ok();
    }

    fn tmp_ext(tag: &str, ext: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("pio_exp_{tag}_{}.{ext}", std::process::id()))
    }

    #[test]
    fn points_export_geojson_and_csv_round_trip() {
        let mut attrs = IndexMap::new();
        attrs.insert("poro".to_string(), vec![0.2, f64::NAN, 0.3]);
        let pts = PointSet {
            coords: vec![[1.0, 2.0, 100.0], [3.0, 4.0, 110.0], [5.0, 6.0, 120.0]],
            attrs,
        };
        // GeoJSON round-trip (NaN → null → NaN).
        let g = tmp_ext("pts_gj", "geojson");
        pts.export_geojson(&g).unwrap();
        let bg = PointSet::load_geojson(&g).unwrap();
        assert_eq!(bg.len(), 3);
        assert_eq!(bg.stats("poro").unwrap().count, 2);
        std::fs::remove_file(&g).ok();
        // CSV round-trip.
        let c = tmp_ext("pts_csv", "csv");
        pts.export_csv(&c).unwrap();
        let bc = PointSet::load_csv(&c, "x", "y", "z").unwrap();
        assert_eq!(bc.len(), 3);
        assert_eq!(bc.stats("poro").unwrap().count, 2);
        std::fs::remove_file(&c).ok();
    }

    #[test]
    fn polygons_export_geojson_round_trip() {
        let rings = vec![vec![
            [0.0, 0.0, 0.0],
            [10.0, 0.0, 0.0],
            [10.0, 10.0, 0.0],
            [0.0, 0.0, 0.0],
        ]];
        let pg = PolygonSet::from_rings(rings);
        let g = tmp_ext("pgn_gj", "geojson");
        pg.export_geojson(&g).unwrap();
        let back = PolygonSet::load_geojson(&g).unwrap();
        assert!(back.contains(3.0, 1.0));
        assert!(!back.contains(-1.0, -1.0));
        std::fs::remove_file(&g).ok();
    }

    #[test]
    fn polygons_round_trip() {
        let rings = vec![vec![
            [0.0, 0.0, 0.0],
            [10.0, 0.0, 0.0],
            [10.0, 10.0, 0.0],
            [0.0, 0.0, 0.0],
        ]];
        let pg = PolygonSet::from_rings(rings);
        let p = tmp("pgn");
        pg.save(&p).unwrap();
        let back = PolygonSet::load(&p).unwrap();
        assert!(back.contains(3.0, 1.0));
        assert!(!back.contains(-1.0, -1.0));
        std::fs::remove_file(&p).ok();
    }
}
