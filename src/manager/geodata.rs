//! `GeoData` — the load-once project substrate.
//!
//! A [`GeoData`] holds a project's surfaces, wells, points, and polygons keyed
//! by name (insertion-ordered [`IndexMap`]s) under a single declared [`Unit`].
//! The loaders ingest a file once and hand back a borrow (`Result<&T>`); named
//! getters and collection views (`surfaces()` / `wells()`) read it back. This is
//! the SPEC's "manager substrate": operations broadcast across a collection via
//! [`WellsView`], never per-item loops in caller code.
//!
//! This module holds the project state + named/collection access + strat hints;
//! the extension-dispatched `load_*` ingest and its well-routing helpers live in
//! the sibling [`loaders`](super::loaders) module.

use crate::core::{PointSet, PolygonSet, Surface, Well};
use crate::foundation::{GeoError, Result, Unit};
use crate::manager::wells_view::WellsView;
use indexmap::IndexMap;

/// A load-once subsurface project: named surfaces, wells, points, and polygons
/// under one declared length [`Unit`].
pub struct GeoData {
    /// The project's length unit; surfaces/wells/points/polygons share it.
    pub unit: Unit,
    pub(crate) surfaces: IndexMap<String, Surface>,
    pub(crate) wells: IndexMap<String, Well>,
    pub(crate) points: IndexMap<String, PointSet>,
    pub(crate) polygons: IndexMap<String, PolygonSet>,
    /// Global lithostratigraphic column (top names, shallow→deep) derived from
    /// the last loaded well-tops file across *all* its wells. Empty until
    /// [`load_well_tops`](GeoData::load_well_tops) runs; pushed down into every
    /// well so `zones()`/`zone_stats()` present in this order.
    pub(crate) strat_order: Vec<String>,
    /// User-supplied soft ordering hints `(above, below)` as raw name tokens
    /// (possibly partial). Applied during `load_well_tops` — resolved to actual
    /// top names, then honoured only where the data leaves the pair unordered.
    pub(crate) strat_hints: Vec<(String, String)>,
    /// Project metadata persisted to a `.pproj` manifest (owner / tags / created;
    /// `modified` is stamped at save time).
    pub(crate) owner: Option<String>,
    pub(crate) tags: Vec<String>,
    pub(crate) created: Option<u64>,
    /// Per-element custom tags (keyed by element name), written into each section
    /// so `export`-by-tag can select a shareable subset.
    pub(crate) element_tags: IndexMap<String, Vec<String>>,
    /// petekSim's opaque model sidecar: `model/<seg>/…` sections held as raw
    /// bytes + a per-section version. petekIO frames/compresses them on save and
    /// hands them back untouched — it never parses their contents.
    pub(crate) model_sections: IndexMap<String, crate::manager::ModelSection>,
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
            strat_order: Vec::new(),
            strat_hints: Vec::new(),
            owner: None,
            tags: Vec::new(),
            created: None,
            element_tags: IndexMap::new(),
            model_sections: IndexMap::new(),
        }
    }

    /// The global lithostratigraphic column (top names, shallow→deep) derived by
    /// the last [`load_well_tops`](GeoData::load_well_tops) across every well in
    /// that file. Empty before any tops are loaded.
    pub fn strat_order(&self) -> &[String] {
        &self.strat_order
    }

    /// Add a soft lithostratigraphic hint: place `above` shallower than `below`.
    /// `above`/`below` may be partial top names (resolved at `load_well_tops`).
    /// A hint is honoured only where the *data* leaves the pair unordered — it
    /// never overrides a strict MD relationship. Add hints **before**
    /// `load_well_tops`. See [`strat_hint`](GeoData::strat_hint) for shorthand.
    pub fn add_strat_hint(&mut self, above: &str, below: &str) {
        self.strat_hints
            .push((above.to_string(), below.to_string()));
    }

    /// Shorthand for [`add_strat_hint`](GeoData::add_strat_hint): `"A < B"` reads
    /// "A above B"; `"A > B"` reads "A below B". Sides may be partial names.
    /// `Err` if the spec carries neither `<` nor `>` or has an empty side.
    pub fn strat_hint(&mut self, spec: &str) -> Result<()> {
        let (above, below) = if let Some((l, r)) = spec.split_once('<') {
            (l.trim(), r.trim())
        } else if let Some((l, r)) = spec.split_once('>') {
            (r.trim(), l.trim())
        } else {
            return Err(GeoError::Parse(format!(
                "strat hint '{spec}' must contain '<' (above) or '>' (below)"
            )));
        };
        if above.is_empty() || below.is_empty() {
            return Err(GeoError::Parse(format!(
                "strat hint '{spec}' has an empty side"
            )));
        }
        self.add_strat_hint(above, below);
        Ok(())
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

    /// All surfaces with their names, in insertion order.
    pub fn surfaces_named(&self) -> impl Iterator<Item = (&str, &Surface)> {
        self.surfaces.iter().map(|(k, v)| (k.as_str(), v))
    }

    /// All polygon sets with their names, in insertion order.
    pub fn polygons_named(&self) -> impl Iterator<Item = (&str, &PolygonSet)> {
        self.polygons.iter().map(|(k, v)| (k.as_str(), v))
    }

    /// A broadcastable, filterable view over all wells (insertion order).
    pub fn wells(&self) -> WellsView<'_> {
        WellsView::new(self.wells.values().collect())
    }
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
