//! `GeoData` — the load-once project substrate.
//!
//! A [`GeoData`] holds a project's surfaces, wells, points, and polygons keyed
//! by name (insertion-ordered [`IndexMap`]s) under a single declared [`Unit`].
//! The loaders ingest a file once and hand back a borrow (`Result<&T>`); named
//! getters and collection views (`surfaces()` / `wells()`) read it back. This is
//! the SPEC's "manager substrate": operations broadcast across a collection via
//! [`WellsView`], never per-item loops in caller code.
//!
//! The loaders dispatch on file extension over the formats the `io`/`core`
//! layers already support; unknown extensions are a typed `GeoError`. See each
//! method for the formats it accepts.

use crate::core::{Log, PointSet, PolygonSet, Station, Surface, Top, TrajectoryInput, Well};
use crate::foundation::{GeoError, Result, Unit};
use crate::manager::wells_view::WellsView;
use indexmap::IndexMap;
use std::path::Path;

/// A load-once subsurface project: named surfaces, wells, points, and polygons
/// under one declared length [`Unit`].
pub struct GeoData {
    /// The project's length unit; surfaces/wells/points/polygons share it.
    pub unit: Unit,
    surfaces: IndexMap<String, Surface>,
    wells: IndexMap<String, Well>,
    points: IndexMap<String, PointSet>,
    polygons: IndexMap<String, PolygonSet>,
}

/// Lower-cased file extension of `path`, or `""` when it has none.
fn ext_of(path: &Path) -> String {
    path.extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
}

impl GeoData {
    /// An empty project in `unit`.
    pub fn new(unit: Unit) -> GeoData {
        GeoData {
            unit,
            surfaces: IndexMap::new(),
            wells: IndexMap::new(),
            points: IndexMap::new(),
            polygons: IndexMap::new(),
        }
    }

    /// Load a surface from `path` and store it under `name`. Reads the IRAP
    /// classic (FIRST) ASCII grid — the surface format the `io` layer supports.
    /// Returns a borrow of the stored surface.
    pub fn load_surface(&mut self, name: &str, path: impl AsRef<Path>) -> Result<&Surface> {
        let path = path.as_ref();
        let surface = match ext_of(path).as_str() {
            "irap" | "gri" | "" => Surface::load_irap_classic(path)?,
            other => {
                return Err(GeoError::Parse(format!(
                    "load_surface: unsupported surface extension '.{other}' for '{}'",
                    path.display()
                )))
            }
        };
        let entry = self.surfaces.entry(name.to_string());
        Ok(entry.or_insert(surface))
    }

    /// Load a well from `files` and store it under `id`, returning a borrow.
    ///
    /// `files` is a directory (the common case — a per-well folder) or a single
    /// file. Each contained file is ingested by extension:
    /// - `*.las` → every non-index curve becomes a [`Log`] on the main bore;
    /// - `*.csv` → formation tops (columns `name`, `md`) on the main bore.
    ///
    /// Other files are ignored. When logs are present a vertical trajectory
    /// spanning their measured-depth range is synthesized so positions resolve
    /// and the deepest top's interval runs to total depth. (There is no well
    /// deviation loader yet; survey ingest lands when `io` grows one.)
    pub fn load_well(
        &mut self,
        id: &str,
        head: (f64, f64),
        kb: f64,
        files: impl AsRef<Path>,
    ) -> Result<&Well> {
        let root = files.as_ref();

        // Gather the files to ingest: a directory's entries (sorted by name for
        // deterministic order), or the single given file.
        let mut paths: Vec<std::path::PathBuf> = if root.is_dir() {
            let mut entries: Vec<_> = std::fs::read_dir(root)?
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.is_file())
                .collect();
            entries.sort();
            entries
        } else {
            vec![root.to_path_buf()]
        };
        paths.retain(|p| p.is_file());

        let mut logs: Vec<Log> = Vec::new();
        let mut tops: Vec<Top> = Vec::new();
        for path in &paths {
            match ext_of(path).as_str() {
                "las" => logs.extend(Log::load_las_all(path)?),
                "csv" => tops.extend(Top::load_csv(path, "name", "md")?),
                _ => {} // ignore unrelated files in a well folder
            }
        }

        let mut well = Well::new(id, head, kb);
        {
            let st = well.sidetrack_mut("");
            if let Some((lo, hi)) = log_md_span(&logs) {
                st.add_trajectory(TrajectoryInput::Stations(vec![
                    Station::new(lo, 0.0, 0.0),
                    Station::new(hi, 0.0, 0.0),
                ]))?;
            }
            for log in logs {
                st.add_log(log);
            }
            if !tops.is_empty() {
                st.add_tops(tops);
            }
        }

        let entry = self.wells.entry(id.to_string());
        Ok(entry.or_insert(well))
    }

    /// Load a point set from `path` and store it under `name`, returning a
    /// borrow. Dispatches on extension: `.geojson` → GeoJSON; `.csv` → headered
    /// CSV with `x`/`y`/`z` columns (other numeric columns → attributes);
    /// `.xyz`/`.irap`/`.dat`/none → RMS plain `X Y Z`.
    pub fn load_points(&mut self, name: &str, path: impl AsRef<Path>) -> Result<&PointSet> {
        let path = path.as_ref();
        let points = match ext_of(path).as_str() {
            "geojson" | "json" => PointSet::load_geojson(path)?,
            "csv" => PointSet::load_csv(path, "x", "y", "z")?,
            "xyz" | "irap" | "dat" | "" => PointSet::load_irap_points(path)?,
            other => {
                return Err(GeoError::Parse(format!(
                    "load_points: unsupported point extension '.{other}' for '{}'",
                    path.display()
                )))
            }
        };
        let entry = self.points.entry(name.to_string());
        Ok(entry.or_insert(points))
    }

    /// Load a polygon set from `path` and store it under `name`, returning a
    /// borrow. Dispatches on extension: `.geojson` → GeoJSON; `.shp` →
    /// shapefile; `.pol`/`.xyz`/`.irap`/none → RMS rings (`999.0` separators).
    pub fn load_polygons(&mut self, name: &str, path: impl AsRef<Path>) -> Result<&PolygonSet> {
        let path = path.as_ref();
        let polygons = match ext_of(path).as_str() {
            "geojson" | "json" => PolygonSet::load_geojson(path)?,
            "shp" => PolygonSet::load_shapefile(path)?,
            "pol" | "xyz" | "irap" | "" => PolygonSet::load_irap_polygons(path)?,
            other => {
                return Err(GeoError::Parse(format!(
                    "load_polygons: unsupported polygon extension '.{other}' for '{}'",
                    path.display()
                )))
            }
        };
        let entry = self.polygons.entry(name.to_string());
        Ok(entry.or_insert(polygons))
    }

    /// The surface stored under `name`, or `None`.
    pub fn surface(&self, name: &str) -> Option<&Surface> {
        self.surfaces.get(name)
    }

    /// The well stored under `id`, or `None`.
    pub fn well(&self, id: &str) -> Option<&Well> {
        self.wells.get(id)
    }

    /// The point set stored under `name`, or `None`.
    pub fn points(&self, name: &str) -> Option<&PointSet> {
        self.points.get(name)
    }

    /// The polygon set stored under `name`, or `None`.
    pub fn polygons(&self, name: &str) -> Option<&PolygonSet> {
        self.polygons.get(name)
    }

    /// All surfaces in insertion order.
    pub fn surfaces(&self) -> impl Iterator<Item = &Surface> {
        self.surfaces.values()
    }

    /// A broadcastable, filterable view over all wells (insertion order).
    pub fn wells(&self) -> WellsView<'_> {
        WellsView::new(self.wells.values().collect())
    }
}

/// The `[min, max]` measured-depth span across all `logs`, or `None` when there
/// is no usable (finite, non-degenerate) range.
fn log_md_span(logs: &[Log]) -> Option<(f64, f64)> {
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    for log in logs {
        let md = log.view();
        let md = md.md();
        if let (Some(&first), Some(&last)) = (md.first(), md.last()) {
            lo = lo.min(first);
            hi = hi.max(last);
        }
    }
    (lo.is_finite() && hi.is_finite() && hi > lo).then_some((lo, hi))
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    const IRAP: &str = "tests/fixtures/simple.irap";
    const WELL_DIR: &str = "tests/fixtures/wells/15_9-A1";
    const LAS: &str = "tests/fixtures/sample.las";
    const XYZ: &str = "tests/fixtures/points.xyz";
    const POL: &str = "tests/fixtures/square.pol";

    #[test]
    fn new_is_empty_and_carries_unit() {
        let geo = GeoData::new(Unit::Feet);
        assert_eq!(geo.unit, Unit::Feet);
        assert_eq!(geo.surfaces().count(), 0);
        assert!(geo.wells().is_empty());
        assert!(geo.surface("nope").is_none());
    }

    #[test]
    fn load_surfaces_named_and_collection() {
        let mut geo = GeoData::new(Unit::Metres);
        geo.load_surface("top", IRAP).unwrap();
        geo.load_surface("base", IRAP).unwrap();
        assert!(geo.surface("top").is_some());
        assert!(geo.surface("base").is_some());
        assert!(geo.surface("missing").is_none()); // miss → None
        assert_eq!(geo.surfaces().count(), 2);
    }

    #[test]
    fn load_points_and_polygons_by_extension() {
        let mut geo = GeoData::new(Unit::Metres);
        geo.load_points("wells_xy", XYZ).unwrap();
        geo.load_polygons("outline", POL).unwrap();
        assert_eq!(geo.points("wells_xy").unwrap().len(), 3);
        assert!(geo.polygons("outline").unwrap().contains(0.5, 0.5));
        assert!(geo.points("nope").is_none());
        assert!(geo.polygons("nope").is_none());
    }

    #[test]
    fn unsupported_extension_errors() {
        let mut geo = GeoData::new(Unit::Metres);
        assert!(geo.load_surface("s", "x.segy").is_err());
    }

    #[test]
    fn load_well_from_directory_attaches_logs_and_tops() {
        let mut geo = GeoData::new(Unit::Metres);
        geo.load_well("15/9-A1", (1200.0, 1500.0), 82.0, WELL_DIR)
            .unwrap();
        let w = geo.well("15/9-A1").unwrap();
        assert_eq!(w.head, (1200.0, 1500.0));
        // Tops attached → Brent interval resolves; NTG log clips to it.
        let stats = w.top("Brent").unwrap().log("NTG").unwrap().stats();
        assert_eq!(stats.count, 5);
        assert_relative_eq!(stats.mean, 0.3, epsilon = 1e-12);
        // Synthesized vertical trajectory → positions resolve.
        let p = w.xyz(2420.0).unwrap();
        assert_relative_eq!(p.x, 1200.0, epsilon = 1e-9);
        assert_relative_eq!(p.z, 2420.0 - 82.0, epsilon = 1e-9);
    }

    #[test]
    fn load_well_from_single_file() {
        let mut geo = GeoData::new(Unit::Metres);
        geo.load_well("only-logs", (0.0, 0.0), 0.0, LAS).unwrap();
        let w = geo.well("only-logs").unwrap();
        assert!(w.log("GR").is_some());
        assert!(w.top("Brent").is_none()); // no tops file → no tops
    }

    #[test]
    fn wells_view_iter_filter_and_tops() {
        let mut geo = GeoData::new(Unit::Metres);
        geo.load_well("15/9-A1", (1200.0, 1500.0), 82.0, WELL_DIR)
            .unwrap();
        geo.load_well("no-tops", (0.0, 0.0), 0.0, LAS).unwrap();

        assert_eq!(geo.wells().iter().count(), 2);
        // Filter on a well predicate.
        let east = geo.wells().filter(|w| w.head.0 > 1000.0);
        assert_eq!(east.len(), 1);
        assert_eq!(east.iter().next().unwrap().id, "15/9-A1");
        // tops() narrows to wells that have the marker.
        let brent = geo.wells().tops("Brent");
        assert_eq!(brent.len(), 1);
        assert_eq!(brent.iter().next().unwrap().id, "15/9-A1");

        // Broadcast-style reduction over the narrowed view.
        let means: Vec<f64> = geo
            .wells()
            .tops("Brent")
            .iter()
            .filter_map(|w| Some(w.top("Brent")?.log("NTG")?.stats().mean))
            .collect();
        assert_eq!(means.len(), 1);
        assert_relative_eq!(means[0], 0.3, epsilon = 1e-12);
    }
}
