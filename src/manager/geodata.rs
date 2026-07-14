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

use crate::core::{
    PointSet, PolygonSet, StructuredMeshSurface, Surface, SurfaceIntersection, TriSurface, Well,
};
use crate::foundation::{GeoError, Result, Unit};
use crate::manager::wells_view::WellsView;
use indexmap::IndexMap;

/// A load-once subsurface project: named surfaces, wells, points, and polygons
/// under one declared length [`Unit`].
pub struct GeoData {
    /// The project's length unit; surfaces/wells/points/polygons share it.
    pub unit: Unit,
    /// Optional user-facing project title. It is deliberately not inferred
    /// from the file path so save/load never changes author intent.
    pub(crate) display_name: Option<String>,
    /// Optional free-text coordinate reference system declaration.
    pub(crate) crs: Option<String>,
    pub(crate) surfaces: IndexMap<String, Surface>,
    /// Explicit-node structured surfaces. Kept separate so the existing Rust
    /// regular-surface API remains source-compatible; names are nevertheless
    /// unique across both surface collections.
    pub(crate) structured_surfaces: IndexMap<String, StructuredMeshSurface>,
    pub(crate) tri_surfaces: IndexMap<String, TriSurface>,
    /// Cross-kind surface order used by project inventory and persistence.
    pub(crate) surface_order: Vec<String>,
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
    /// Generic project assets (viewer templates today, other provider-owned
    /// values later). They are persisted separately from domain/model sections
    /// and retain their envelope + payload bytes exactly.
    pub(crate) assets: IndexMap<String, crate::manager::ProjectAsset>,
    /// Opt-in curve-mnemonic canonicalization applied at `load_well` time. When
    /// `Some`, each loaded log's mnemonic is passed through
    /// [`canonical_mnemonic_with`](crate::analysis::canonical_mnemonic_with)
    /// (the user map first, then the built-in table + vintage `_YYYY` strip), so
    /// e.g. `PHIE_2025` → `PHIE`. `None` (default) preserves raw mnemonics.
    pub(crate) curve_aliases: Option<crate::analysis::NameMap>,
}

enum ProjectSurfaceRef<'a> {
    Regular(&'a Surface),
    Structured(&'a StructuredMeshSurface),
    Tri(&'a TriSurface),
}

#[derive(PartialEq)]
struct SurfaceGeometrySignature {
    nodes: Vec<[f64; 2]>,
    triangles: Vec<[u32; 3]>,
    wireframe: Vec<[u32; 2]>,
    edge_rings: Vec<Vec<[f64; 3]>>,
    labels: Vec<Option<crate::WalkLabel>>,
}

impl ProjectSurfaceRef<'_> {
    fn geometry_signature(&self) -> Result<SurfaceGeometrySignature> {
        let owned;
        let tri = match self {
            ProjectSurfaceRef::Regular(surface) => {
                owned = surface.to_tri_surface()?;
                &owned
            }
            ProjectSurfaceRef::Structured(surface) => {
                owned = surface.to_tri_surface()?;
                &owned
            }
            ProjectSurfaceRef::Tri(surface) => surface,
        };
        Ok(SurfaceGeometrySignature {
            nodes: tri.shell().nodes().to_vec(),
            triangles: tri.shell().triangles().to_vec(),
            wireframe: tri.shell().wireframe_edges(None),
            edge_rings: tri.shell().edge().rings(),
            labels: tri.shell().labels().to_vec(),
        })
    }

    fn same_geometry(&self, other: &ProjectSurfaceRef<'_>) -> Result<bool> {
        match (self, other) {
            (ProjectSurfaceRef::Regular(left), ProjectSurfaceRef::Regular(right)) => {
                Ok(left.geom == right.geom)
            }
            (ProjectSurfaceRef::Structured(left), ProjectSurfaceRef::Structured(right)) => {
                Ok(left.ncol() == right.ncol()
                    && left.nrow() == right.nrow()
                    && left.x() == right.x()
                    && left.y() == right.y()
                    && left.edge().rings() == right.edge().rings())
            }
            (ProjectSurfaceRef::Regular(regular), ProjectSurfaceRef::Structured(structured))
            | (ProjectSurfaceRef::Structured(structured), ProjectSurfaceRef::Regular(regular)) => {
                if (regular.geom.ncol, regular.geom.nrow) != (structured.ncol(), structured.nrow())
                {
                    return Ok(false);
                }
                let same_nodes = (0..regular.geom.nrow).all(|j| {
                    (0..regular.geom.ncol).all(|i| {
                        let (x, y) = regular.geom.node_xy(i, j);
                        structured.x()[[i, j]] == x && structured.y()[[i, j]] == y
                    })
                });
                let regular_edge = crate::PolygonSet::from_grid_geometry(&regular.geom);
                Ok(same_nodes && regular_edge.rings() == structured.edge().rings())
            }
            _ => Ok(self.geometry_signature()? == other.geometry_signature()?),
        }
    }
}

/// One persisted formation-pick row aggregated from a well bore.
#[derive(Debug, Clone, PartialEq)]
pub struct WellTopRow {
    pub well: String,
    pub bore: String,
    pub md: f64,
    pub xyz: Option<crate::foundation::Point3>,
}
impl GeoData {
    /// An empty project in `unit`.
    pub fn new(unit: Unit) -> GeoData {
        GeoData {
            unit,
            display_name: None,
            crs: None,
            surfaces: IndexMap::new(),
            structured_surfaces: IndexMap::new(),
            tri_surfaces: IndexMap::new(),
            surface_order: Vec::new(),
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
            assets: IndexMap::new(),
            curve_aliases: None,
        }
    }

    /// Enable curve-mnemonic canonicalization for subsequent `load_well` calls:
    /// each loaded log's mnemonic is mapped to canonical via `aliases` first
    /// (raw vendor mnemonic → canonical, e.g. `PHIE_2025` → `PHIE`), then the
    /// built-in table + vintage `_YYYY` strip. Pass an empty [`NameMap`] to get
    /// pure auto-canonicalization (vintage strip + table) with no explicit
    /// mappings. Off by default (raw mnemonics preserved).
    pub fn set_curve_aliases(&mut self, aliases: crate::analysis::NameMap) {
        self.curve_aliases = Some(aliases);
    }

    /// Load a well with a **per-call** curve-alias map — declarative, non-sticky:
    /// `aliases` is applied to this load only (the project's own
    /// [`set_curve_aliases`](GeoData::set_curve_aliases) state is preserved and
    /// restored around the call, and `None` means no canonicalization for this
    /// load regardless of prior sticky state). The value behind the Python
    /// `IngestSpec` applied at `load_well(..., ingest=)`; returns `Ok(())` (the
    /// loaded well is read back by id).
    pub fn load_well_with(
        &mut self,
        id: &str,
        head: (f64, f64),
        kb: f64,
        files: impl AsRef<std::path::Path>,
        aliases: Option<&crate::analysis::NameMap>,
    ) -> Result<()> {
        let saved = self.curve_aliases.clone();
        self.curve_aliases = aliases.cloned();
        let r = self.load_well(id, head, kb, files).map(|_| ());
        self.curve_aliases = saved;
        r
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
        let mut h = crate::analysis::StratHints::new();
        h.push_spec(spec)?;
        self.add_strat_hints(&h);
        Ok(())
    }

    /// Apply a declarative [`StratHints`](crate::analysis::StratHints) value:
    /// append its `(above, below)` token pairs to the project's soft ordering
    /// hints (honoured by the next `load_well_tops`, data always winning). The
    /// declarative counterpart of [`add_strat_hint`](GeoData::add_strat_hint) /
    /// [`strat_hint`](GeoData::strat_hint) — the value carried by the Python
    /// `IngestSpec`, applied at load.
    pub fn add_strat_hints(&mut self, hints: &crate::analysis::StratHints) {
        self.strat_hints.extend(hints.pairs().iter().cloned());
    }
    /// The surface stored under `name`, or `None`.
    pub fn surface(&self, name: &str) -> Option<&Surface> {
        self.surfaces.get(name)
    }

    /// The structured mesh surface stored under `name`, or `None`.
    pub fn structured_surface(&self, name: &str) -> Option<&StructuredMeshSurface> {
        self.structured_surfaces.get(name)
    }

    /// The triangulated surface stored under `name`, or `None`.
    pub fn tri_surface(&self, name: &str) -> Option<&TriSurface> {
        self.tri_surfaces.get(name)
    }

    /// Explicitly replace an existing project surface with a detached regular surface.
    pub fn replace_surface(&mut self, name: &str, surface: Surface) -> Result<&Surface> {
        self.check_replacement_geometry(name, ProjectSurfaceRef::Regular(&surface))?;
        self.remove_surface_value(name);
        self.surfaces.insert(name.to_string(), surface);
        Ok(self.surfaces.get(name).expect("just inserted"))
    }

    /// Explicitly replace an existing project surface with a detached structured surface.
    pub fn replace_structured_surface(
        &mut self,
        name: &str,
        surface: StructuredMeshSurface,
    ) -> Result<&StructuredMeshSurface> {
        self.check_replacement_geometry(name, ProjectSurfaceRef::Structured(&surface))?;
        self.remove_surface_value(name);
        self.structured_surfaces.insert(name.to_string(), surface);
        Ok(self.structured_surfaces.get(name).expect("just inserted"))
    }

    /// Explicitly replace an existing project surface with a detached triangulated surface.
    pub fn replace_tri_surface(&mut self, name: &str, surface: TriSurface) -> Result<&TriSurface> {
        self.check_replacement_geometry(name, ProjectSurfaceRef::Tri(&surface))?;
        self.remove_surface_value(name);
        self.tri_surfaces.insert(name.to_string(), surface);
        Ok(self.tri_surfaces.get(name).expect("just inserted"))
    }

    fn check_replacement_geometry(
        &self,
        name: &str,
        replacement: ProjectSurfaceRef<'_>,
    ) -> Result<()> {
        let current = if let Some(surface) = self.surfaces.get(name) {
            ProjectSurfaceRef::Regular(surface)
        } else if let Some(surface) = self.structured_surfaces.get(name) {
            ProjectSurfaceRef::Structured(surface)
        } else if let Some(surface) = self.tri_surfaces.get(name) {
            ProjectSurfaceRef::Tri(surface)
        } else {
            return Err(GeoError::NotFound(format!("surface '{name}'")));
        };
        if !current.same_geometry(&replacement)? {
            return Err(GeoError::GeometryMismatch(format!(
                "replace_surface('{name}'): replacement geometry/topology differs from the stored surface"
            )));
        }
        Ok(())
    }

    fn remove_surface_value(&mut self, name: &str) {
        self.surfaces.shift_remove(name);
        self.structured_surfaces.shift_remove(name);
        self.tri_surfaces.shift_remove(name);
    }

    /// Rename a stored surface.
    pub fn rename_surface(&mut self, old: &str, new: &str) -> Result<()> {
        if old == new {
            return Ok(());
        }
        if self.surfaces.contains_key(new)
            || self.structured_surfaces.contains_key(new)
            || self.tri_surfaces.contains_key(new)
        {
            return Err(GeoError::Parse(format!(
                "rename_surface: destination '{new}' already exists"
            )));
        }
        if self.surfaces.contains_key(old) {
            rename_key(&mut self.surfaces, old, new, "surface")?;
        } else if self.structured_surfaces.contains_key(old) {
            rename_key(&mut self.structured_surfaces, old, new, "surface")?;
        } else if self.tri_surfaces.contains_key(old) {
            rename_key(&mut self.tri_surfaces, old, new, "surface")?;
        } else {
            return Err(GeoError::NotFound(format!("surface '{old}'")));
        }
        if let Some(name) = self
            .surface_order
            .iter_mut()
            .find(|name| name.as_str() == old)
        {
            *name = new.to_string();
        }
        rename_element_tags(&mut self.element_tags, old, new);
        Ok(())
    }

    /// Delete a stored surface. Returns whether anything was removed.
    pub fn delete_surface(&mut self, name: &str) -> bool {
        let removed = self.surfaces.shift_remove(name).is_some()
            || self.structured_surfaces.shift_remove(name).is_some()
            || self.tri_surfaces.shift_remove(name).is_some();
        if removed {
            self.surface_order.retain(|stored| stored != name);
            self.element_tags.shift_remove(name);
        }
        removed
    }

    /// The well stored under `id`, or `None`.
    pub fn well(&self, id: &str) -> Option<&Well> {
        self.wells.get(id)
    }

    /// Rename a stored well.
    pub fn rename_well(&mut self, old: &str, new: &str) -> Result<()> {
        rename_key(&mut self.wells, old, new, "well")?;
        rename_element_tags(&mut self.element_tags, old, new);
        Ok(())
    }

    /// Delete a stored well. Returns whether anything was removed.
    pub fn delete_well(&mut self, id: &str) -> bool {
        let removed = self.wells.shift_remove(id).is_some();
        if removed {
            self.element_tags.shift_remove(id);
        }
        removed
    }

    /// Mutable access to the well stored under `id`, or `None` — for in-place
    /// selection such as [`Well::set_default_bore`](crate::Well::set_default_bore)
    /// on a multi-bore well (the Python `Well.set_default_bore` routes here).
    pub fn well_mut(&mut self, id: &str) -> Option<&mut Well> {
        self.wells.get_mut(id)
    }

    /// Distinct formation-top names across all bores, in first-seen order.
    pub fn well_top_names(&self) -> Vec<String> {
        let mut names = Vec::new();
        for well in self.wells.values() {
            for bore in well.sidetracks() {
                for top in bore.tops() {
                    if !names
                        .iter()
                        .any(|name: &String| name.eq_ignore_ascii_case(&top.name))
                    {
                        names.push(top.name.clone());
                    }
                }
            }
        }
        names
    }

    /// All persisted picks named `name`, with recomputed XYZ where positioned.
    pub fn well_top_set(&self, name: &str) -> Vec<WellTopRow> {
        let mut rows = Vec::new();
        for well in self.wells.values() {
            for bore in well.sidetracks() {
                for top in bore
                    .tops()
                    .filter(|top| top.name.eq_ignore_ascii_case(name))
                {
                    rows.push(WellTopRow {
                        well: well.id.clone(),
                        bore: bore.label.clone(),
                        md: top.md,
                        xyz: bore.xyz(top.md),
                    });
                }
            }
        }
        rows
    }

    /// Remove a named formation horizon globally. Returns picks removed.
    pub fn delete_well_top(&mut self, name: &str) -> usize {
        let mut removed = 0;
        for well in self.wells.values_mut() {
            for label in well.bores().map(str::to_string).collect::<Vec<_>>() {
                let bore = well.sidetrack_mut(&label);
                let before = bore.tops().count();
                bore.retain_tops_except(name);
                removed += before - bore.tops().count();
            }
        }
        removed
    }

    /// Atomically replace the complete named horizon with one validated hit per
    /// bore. Validation performs no mutation; the subsequent replacement pass
    /// is infallible and removes stale picks outside the hit set.
    pub fn replace_well_top_set(&mut self, name: &str, hits: &[SurfaceIntersection]) -> Result<()> {
        let name = name.trim();
        if name.is_empty() {
            return Err(GeoError::Parse("well-top name cannot be empty".into()));
        }
        let mut keys = std::collections::HashSet::new();
        for hit in hits {
            let well_id = hit.well.as_deref().ok_or_else(|| {
                GeoError::Parse("project well-top assignment needs hit.well identity".into())
            })?;
            let bore_label = hit.bore.as_deref().ok_or_else(|| {
                GeoError::Parse("project well-top assignment needs hit.bore identity".into())
            })?;
            if !keys.insert((well_id.to_string(), bore_label.to_string())) {
                return Err(GeoError::Parse(format!(
                    "well-top assignment has multiple hits for {well_id}/{bore_label}; select one crossing per bore"
                )));
            }
            let well = self.wells.get(well_id).ok_or_else(|| {
                GeoError::NotFound(format!("intersection well '{well_id}' in this project"))
            })?;
            let bore = well.sidetrack(bore_label).ok_or_else(|| {
                GeoError::NotFound(format!(
                    "intersection bore '{bore_label}' on well '{well_id}'"
                ))
            })?;
            bore.validate_intersection(hit)?;
        }

        self.delete_well_top(name);
        for hit in hits {
            let well_id = hit.well.as_deref().expect("validated well identity");
            let bore_label = hit.bore.as_deref().expect("validated bore identity");
            self.wells
                .get_mut(well_id)
                .expect("validated well")
                .sidetrack_mut(bore_label)
                .push_top_validated(name.to_string(), hit.md);
        }
        Ok(())
    }

    /// The point set stored under `name`, or `None`.
    pub fn points(&self, name: &str) -> Option<&PointSet> {
        self.points.get(name)
    }

    /// Rename a stored point set.
    pub fn rename_points(&mut self, old: &str, new: &str) -> Result<()> {
        rename_key(&mut self.points, old, new, "point set")?;
        rename_element_tags(&mut self.element_tags, old, new);
        Ok(())
    }

    /// Delete a stored point set. Returns whether anything was removed.
    pub fn delete_points(&mut self, name: &str) -> bool {
        let removed = self.points.shift_remove(name).is_some();
        if removed {
            self.element_tags.shift_remove(name);
        }
        removed
    }

    /// The polygon set stored under `name`, or `None`.
    pub fn polygons(&self, name: &str) -> Option<&PolygonSet> {
        self.polygons.get(name)
    }

    /// Rename a stored polygon set.
    pub fn rename_polygons(&mut self, old: &str, new: &str) -> Result<()> {
        rename_key(&mut self.polygons, old, new, "polygon set")?;
        rename_element_tags(&mut self.element_tags, old, new);
        Ok(())
    }

    /// Delete a stored polygon set. Returns whether anything was removed.
    pub fn delete_polygons(&mut self, name: &str) -> bool {
        let removed = self.polygons.shift_remove(name).is_some();
        if removed {
            self.element_tags.shift_remove(name);
        }
        removed
    }

    /// All surfaces in insertion order.
    pub fn surfaces(&self) -> impl Iterator<Item = &Surface> {
        self.surfaces.values()
    }

    /// All surfaces with their names, in insertion order.
    pub fn surfaces_named(&self) -> impl Iterator<Item = (&str, &Surface)> {
        self.surfaces.iter().map(|(k, v)| (k.as_str(), v))
    }

    /// All structured mesh surfaces in insertion order.
    pub fn structured_surfaces(&self) -> impl Iterator<Item = &StructuredMeshSurface> {
        self.structured_surfaces.values()
    }

    /// All structured mesh surfaces with names, in insertion order.
    pub fn structured_surfaces_named(
        &self,
    ) -> impl Iterator<Item = (&str, &StructuredMeshSurface)> {
        self.structured_surfaces
            .iter()
            .map(|(k, v)| (k.as_str(), v))
    }

    /// All triangulated surfaces with names, in insertion order.
    pub fn tri_surfaces_named(&self) -> impl Iterator<Item = (&str, &TriSurface)> {
        self.tri_surfaces.iter().map(|(k, v)| (k.as_str(), v))
    }

    /// All polygon sets with their names, in insertion order.
    pub fn polygons_named(&self) -> impl Iterator<Item = (&str, &PolygonSet)> {
        self.polygons.iter().map(|(k, v)| (k.as_str(), v))
    }

    /// A broadcastable, filterable view over all wells (insertion order).
    pub fn wells(&self) -> WellsView<'_> {
        WellsView::new(self.wells.values().collect())
    }

    /// A borrowed view over the requested well ids, preserving request order.
    /// Missing ids are omitted; primarily used by the Python filtered view.
    #[doc(hidden)]
    pub fn wells_by_ids(&self, ids: &[String]) -> WellsView<'_> {
        WellsView::new(ids.iter().filter_map(|id| self.wells.get(id)).collect())
    }

    /// All wells with ids, in insertion order.
    pub fn wells_named(&self) -> impl Iterator<Item = (&str, &Well)> {
        self.wells.iter().map(|(id, well)| (id.as_str(), well))
    }
}

fn rename_key<T>(map: &mut IndexMap<String, T>, old: &str, new: &str, kind: &str) -> Result<()> {
    if old == new {
        return Ok(());
    }
    if map.contains_key(new) {
        return Err(GeoError::Parse(format!(
            "rename_{kind}: destination '{new}' already exists"
        )));
    }
    let Some(value) = map.shift_remove(old) else {
        return Err(GeoError::NotFound(format!("{kind} '{old}'")));
    };
    map.insert(new.to_string(), value);
    Ok(())
}

fn rename_element_tags(tags: &mut IndexMap<String, Vec<String>>, old: &str, new: &str) {
    if old == new {
        return;
    }
    if let Some(value) = tags.shift_remove(old) {
        tags.insert(new.to_string(), value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{GeometryEdge, GridGeometry, MeshShell};
    use approx::assert_relative_eq;
    use ndarray::Array2;

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
    fn explicit_surface_replacement_preserves_geometry_and_allows_level_changes() {
        let mut geo = GeoData::new(Unit::Metres);
        geo.load_surface("top", IRAP).unwrap();
        let mut detached = geo.surface("top").unwrap().clone();
        detached
            .set_attr(
                "facies",
                Array2::from_elem((detached.geom.ncol, detached.geom.nrow), 1.0),
            )
            .unwrap();
        assert!(geo.surface("top").unwrap().attr("facies").is_none());
        geo.replace_surface("top", detached).unwrap();
        assert!(geo.surface("top").unwrap().attr("facies").is_some());

        let structured = geo.surface("top").unwrap().to_structured_mesh().unwrap();
        geo.replace_structured_surface("top", structured).unwrap();
        assert!(geo.surface("top").is_none());
        assert!(geo.structured_surface("top").is_some());

        let tri = geo
            .structured_surface("top")
            .unwrap()
            .to_tri_surface()
            .unwrap();
        geo.replace_tri_surface("top", tri).unwrap();
        assert!(geo.structured_surface("top").is_none());
        assert!(geo.tri_surface("top").is_some());

        let wrong = Surface::constant(
            GridGeometry {
                xori: 0.0,
                yori: 0.0,
                xinc: 1.0,
                yinc: 1.0,
                ncol: 2,
                nrow: 2,
                rotation_deg: 0.0,
                yflip: false,
            },
            1.0,
        );
        assert!(matches!(
            geo.replace_surface("top", wrong),
            Err(GeoError::GeometryMismatch(_))
        ));
    }

    #[test]
    fn degenerate_regular_and_structured_surfaces_replace_and_persist_natively() {
        for (tag, ncol, nrow) in [("one", 1, 1), ("column", 1, 4), ("row", 4, 1)] {
            let geom = GridGeometry {
                xori: 100.0,
                yori: 200.0,
                xinc: 10.0,
                yinc: 20.0,
                ncol,
                nrow,
                rotation_deg: 15.0,
                yflip: false,
            };
            let source = Surface::constant(geom, -1800.0);
            let irap = std::env::temp_dir().join(format!(
                "petekio_degenerate_{tag}_{}.irap",
                std::process::id()
            ));
            source.save_irap_classic(&irap).unwrap();

            let mut geo = GeoData::new(Unit::Metres);
            geo.load_surface("top", &irap).unwrap();
            let mut wrong_geom = geo.surface("top").unwrap().geom.clone();
            wrong_geom.xori += 1.0;
            let wrong_regular = Surface::constant(wrong_geom, -1800.0);
            let wrong_structured = wrong_regular.to_structured_mesh().unwrap();
            assert!(matches!(
                geo.replace_surface("top", wrong_regular),
                Err(GeoError::GeometryMismatch(_))
            ));
            geo.replace_surface("top", geo.surface("top").unwrap().clone())
                .unwrap();

            let structured = geo.surface("top").unwrap().to_structured_mesh().unwrap();
            geo.replace_structured_surface("top", structured).unwrap();
            assert!(matches!(
                geo.replace_structured_surface("top", wrong_structured),
                Err(GeoError::GeometryMismatch(_))
            ));
            geo.replace_structured_surface("top", geo.structured_surface("top").unwrap().clone())
                .unwrap();

            let project = std::env::temp_dir().join(format!(
                "petekio_degenerate_{tag}_{}.pproj",
                std::process::id()
            ));
            geo.save(&project).unwrap();
            let mut reopened = GeoData::open(&project).unwrap();
            let persisted = reopened.structured_surface("top").unwrap();
            assert_eq!((persisted.ncol(), persisted.nrow()), (ncol, nrow));
            reopened
                .replace_structured_surface("top", persisted.clone())
                .unwrap();

            std::fs::remove_file(irap).ok();
            std::fs::remove_file(project).ok();
        }
    }

    #[test]
    fn replacement_rejects_structured_boundaries_and_tri_shell_details() {
        let mut coords = Vec::new();
        let mut columns = Vec::new();
        let mut rows = Vec::new();
        for j in 0..4 {
            for i in 0..4 {
                let z = if i > 1 && j > 1 {
                    f64::NAN
                } else {
                    100.0 + i as f64 + j as f64
                };
                coords.push([i as f64, j as f64, z]);
                columns.push((i + 1) as f64);
                rows.push((j + 1) as f64);
            }
        }
        let mut attrs = IndexMap::new();
        attrs.insert("column".to_string(), columns);
        attrs.insert("row".to_string(), rows);
        let points = PointSet::from_parts(coords, attrs);
        let occupied = points
            .to_structured_surface(1e-6, GeometryEdge::Occupied)
            .unwrap();
        let full_rect = points
            .to_structured_surface(1e-6, GeometryEdge::FullRect)
            .unwrap();
        assert_eq!(occupied.x(), full_rect.x());
        assert_eq!(occupied.y(), full_rect.y());
        assert_ne!(occupied.edge().rings(), full_rect.edge().rings());

        let mut geo = GeoData::new(Unit::Metres);
        geo.structured_surfaces
            .insert("top".into(), occupied.clone());
        geo.surface_order.push("top".into());
        assert!(matches!(
            geo.replace_structured_surface("top", full_rect.clone()),
            Err(GeoError::GeometryMismatch(_))
        ));

        let canonical_tri = occupied.to_tri_surface().unwrap();
        geo.replace_tri_surface("top", canonical_tri).unwrap();
        let current = geo.tri_surface("top").unwrap();

        let altered_edge_shell = MeshShell::new(
            current.shell().nodes().to_vec(),
            current.shell().triangles().to_vec(),
            current.shell().wireframe_edges(None),
            full_rect.edge().clone(),
            current.shell().labels().to_vec(),
        )
        .unwrap();
        let altered_edge = TriSurface::from_shell(
            std::sync::Arc::new(altered_edge_shell),
            current.values().to_vec(),
        )
        .unwrap();
        assert!(matches!(
            geo.replace_tri_surface("top", altered_edge),
            Err(GeoError::GeometryMismatch(_))
        ));

        let current = geo.tri_surface("top").unwrap();
        let mut altered_wireframe = current.shell().wireframe_edges(None);
        altered_wireframe.pop();
        let altered_wireframe_shell = MeshShell::new(
            current.shell().nodes().to_vec(),
            current.shell().triangles().to_vec(),
            altered_wireframe,
            current.edge().clone(),
            current.shell().labels().to_vec(),
        )
        .unwrap();
        let altered_wireframe = TriSurface::from_shell(
            std::sync::Arc::new(altered_wireframe_shell),
            current.values().to_vec(),
        )
        .unwrap();
        assert!(matches!(
            geo.replace_tri_surface("top", altered_wireframe),
            Err(GeoError::GeometryMismatch(_))
        ));
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
        assert_relative_eq!(p.z, 82.0 - 2420.0, epsilon = 1e-9); // negative-down elevation
        assert_relative_eq!(w.tvd(2420.0).unwrap(), 2420.0 - 82.0, epsilon = 1e-9);
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
