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

use crate::core::{
    Log, LogKind, PointSet, PolygonSet, Station, Surface, Top, TrajectoryInput, Well,
};
use crate::foundation::{GeoError, Point3, Result, Unit};
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
    /// Global lithostratigraphic column (top names, shallow→deep) derived from
    /// the last loaded well-tops file across *all* its wells. Empty until
    /// [`load_well_tops`](GeoData::load_well_tops) runs; pushed down into every
    /// well so `zones()`/`zone_stats()` present in this order.
    strat_order: Vec<String>,
    /// User-supplied soft ordering hints `(above, below)` as raw name tokens
    /// (possibly partial). Applied during `load_well_tops` — resolved to actual
    /// top names, then honoured only where the data leaves the pair unordered.
    strat_hints: Vec<(String, String)>,
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
            strat_order: Vec::new(),
            strat_hints: Vec::new(),
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
    /// `files` is a directory or a single file. A directory is walked
    /// **recursively** (so a Petrel export tree with separate `Paths/`/`Logs/`
    /// subdirs works, not just a flat folder); when filenames carry the well id
    /// (`99_9-1_A.wellpath`), only this well's files are taken (others sharing the
    /// tree are skipped). Each file is ingested by extension:
    /// - `*.wellpath` → a **positioned** trajectory; one bore (sidetrack) per
    ///   file, labelled by its filename stem minus the shared prefix
    ///   (`99_9-1_A`/`99_9-1_ST2` → bores `A`/`ST2`). The header's wellhead XY /
    ///   KB / CRS are taken as authoritative.
    /// - `*.las` → every non-index curve becomes a [`Log`], routed to the bore
    ///   whose label appears in the filename (else the main bore). A LAS that
    ///   fails to parse (an unsupported variant) is skipped, not fatal.
    /// - `*.csv` → formation tops (columns `name`, `md`) on the main bore.
    ///
    /// With **no** `.wellpath`, a single main bore is built with a vertical
    /// trajectory synthesized over the logs' measured-depth range.
    pub fn load_well(
        &mut self,
        id: &str,
        head: (f64, f64),
        kb: f64,
        files: impl AsRef<Path>,
    ) -> Result<&Well> {
        let root = files.as_ref();

        // Gather files to ingest. A directory is walked **recursively** (so a
        // Petrel export tree with separate `Paths/`/`Logs/` subdirs works, not
        // just a flat per-well folder); a single file is taken as-is.
        let mut paths: Vec<std::path::PathBuf> = if root.is_dir() {
            let mut entries = Vec::new();
            collect_files(root, &mut entries)?;
            entries.sort();
            entries
        } else {
            vec![root.to_path_buf()]
        };
        paths.retain(|p| p.is_file());

        // In a shared tree the files are well-id-named (`99_9-1_A.wellpath`);
        // keep only this well's. If no filename carries the id (a flat folder
        // with generic names like `sample.las`), every file belongs to the well.
        let id_key = normalize_id(id);
        if paths.iter().any(|p| file_matches_id(p, &id_key)) {
            paths.retain(|p| file_matches_id(p, &id_key));
        }

        let wellpaths: Vec<_> = paths
            .iter()
            .filter(|p| ext_of(p) == "wellpath")
            .cloned()
            .collect();
        let las: Vec<_> = paths
            .iter()
            .filter(|p| ext_of(p) == "las")
            .cloned()
            .collect();
        let mut tops: Vec<Top> = Vec::new();
        for path in &paths {
            if ext_of(path) == "csv" {
                tops.extend(Top::load_csv(path, "name", "md")?);
            }
        }

        let mut well = Well::new(id, head, kb);

        if wellpaths.is_empty() {
            // No survey files → single main bore with a synthesized vertical
            // trajectory spanning the logs' MD range.
            let mut logs = Vec::new();
            for p in &las {
                // Skip a LAS that fails to parse (e.g. an unsupported variant)
                // rather than aborting the whole well.
                logs.extend(load_tagged_logs(p).ok().into_iter().flatten());
            }
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
        } else {
            // One bore per .wellpath (label = filename stem minus the shared
            // prefix); positioned trajectory used directly (z = TVD − kb).
            let labels = bore_labels(&wellpaths);
            for (i, (wp_path, label)) in labels.iter().enumerate() {
                let wp = crate::io::wellpath::load(wp_path)?;
                if i == 0 {
                    // The .wellpath header is authoritative for the wellhead datum.
                    well.head = wp.head;
                    well.kb = wp.kb;
                    if let Some(c) = &wp.crs {
                        well.set_crs(c.clone());
                    }
                }
                let rows: Vec<(Station, Point3)> = wp
                    .rows
                    .iter()
                    .map(|r| {
                        (
                            Station::new(r.md, r.inc_deg, r.azi_deg),
                            Point3::new(r.x, r.y, r.tvd - wp.kb),
                        )
                    })
                    .collect();
                well.sidetrack_mut(label)
                    .add_trajectory(TrajectoryInput::PositionedSurvey(rows))?;
            }
            // Route each LAS to the bore whose label appears in its filename
            // (fallback: the main bore).
            let label_list: Vec<String> = labels.iter().map(|(_, l)| l.clone()).collect();
            for p in &las {
                let bore = route_bore(p, &label_list);
                let st = well.sidetrack_mut(&bore);
                // Skip a LAS that fails to parse rather than aborting the well.
                for log in load_tagged_logs(p).ok().into_iter().flatten() {
                    st.add_log(log);
                }
            }
        }
        // Tops are well-level here (CSV) → main bore. (Petrel per-well tops land
        // in a later phase.)
        if !tops.is_empty() {
            well.sidetrack_mut("").add_tops(tops);
        }

        let entry = self.wells.entry(id.to_string());
        Ok(entry.or_insert(well))
    }

    /// Load a multi-well **Petrel well-tops** file and distribute each pick to
    /// the matching already-loaded well + bore. The record's `Well` field is
    /// matched to a loaded well id (exact, or the id is a separator-delimited
    /// prefix — `"99/9-1 B"` → well `99/9-1`, bore `B` if that bore exists, else
    /// the main bore). Only `Type == Horizon` picks are taken (lithostratigraphy);
    /// `Other` picks (fluid contacts OWC/GOC/FWL) and unknown-well records are
    /// skipped. Returns the number of tops assigned. (Load wells *before* tops.)
    ///
    /// Side effect: derives the project's **global lithostratigraphic column**
    /// ([`strat_order`](GeoData::strat_order)) from *every* well's Horizon picks
    /// in the file — including wells not loaded into the project — and pushes it
    /// into each loaded well, so `zones()`/`zone_stats()` then present zones in
    /// that order. A well that develops a marker thus resolves an order a well
    /// where it pinches out (zero thickness) cannot.
    pub fn load_well_tops(&mut self, path: impl AsRef<Path>) -> Result<usize> {
        let recs = crate::io::petrel_tops::load(path.as_ref())?;

        // Pre-pass over ALL Horizon picks (every well in the file, loaded or
        // not) → one (md, name) sequence per well → the merged global column.
        // Built before the loaded-well filter below.
        let order = {
            let mut by_well: IndexMap<&str, Vec<(f64, &str)>> = IndexMap::new();
            let mut names: Vec<&str> = Vec::new();
            for r in &recs {
                if r.kind.eq_ignore_ascii_case("Horizon") {
                    by_well
                        .entry(r.well.as_str())
                        .or_default()
                        .push((r.md, r.surface.as_str()));
                    if !names.contains(&r.surface.as_str()) {
                        names.push(r.surface.as_str());
                    }
                }
            }
            // Resolve each (partial) hint token to an actual top name; a bad
            // token errors here rather than silently doing nothing.
            let resolved: Vec<(String, String)> = self
                .strat_hints
                .iter()
                .map(|(a, b)| Ok((resolve_top_name(a, &names)?, resolve_top_name(b, &names)?)))
                .collect::<Result<_>>()?;
            let hints: Vec<(&str, &str)> = resolved
                .iter()
                .map(|(a, b)| (a.as_str(), b.as_str()))
                .collect();
            let seqs: Vec<Vec<(f64, &str)>> = by_well.into_values().collect();
            crate::algorithms::wells::merge_strat_order(&seqs, &hints)
        };

        // Distribute each Horizon pick to the matching loaded well + bore.
        let ids: Vec<String> = self.wells.keys().cloned().collect();
        let mut added = 0;
        for r in recs {
            // Only lithostratigraphic picks define zones; skip `Other` (fluid
            // contacts OWC/GOC/FWL, etc. — not stratigraphy).
            if !r.kind.eq_ignore_ascii_case("Horizon") {
                continue;
            }
            let Some(id) = ids.iter().find(|id| well_name_matches(id, &r.well)) else {
                continue;
            };
            let suffix = bore_suffix(id, &r.well);
            let well = self.wells.get_mut(id).expect("id came from this map");
            let label = if well.sidetrack(&suffix).is_some() {
                suffix
            } else {
                String::new()
            };
            well.sidetrack_mut(&label)
                .add_tops(vec![Top::new(r.surface, r.md)]);
            added += 1;
        }

        // Push the column into every loaded well, then record it on the project.
        for well in self.wells.values_mut() {
            well.set_strat_order(&order);
        }
        self.strat_order = order;
        Ok(added)
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

/// Pair each `.wellpath` with a bore label = its filename stem minus the longest
/// `_`-delimited prefix shared by all the stems (so `99_9-1_A`/`99_9-1_ST2` →
/// `A`/`ST2`). A single wellpath, or no shared prefix, → the main bore `""`.
fn bore_labels(wellpaths: &[std::path::PathBuf]) -> Vec<(std::path::PathBuf, String)> {
    let stems: Vec<String> = wellpaths
        .iter()
        .map(|p| {
            p.file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string()
        })
        .collect();
    let prefix = shared_underscore_prefix(&stems);
    wellpaths
        .iter()
        .zip(&stems)
        .map(|(p, stem)| {
            let label = stem.strip_prefix(&prefix).unwrap_or(stem).to_string();
            (p.clone(), label)
        })
        .collect()
}

/// The longest prefix ending at a `_` boundary shared by every stem (`""` if
/// fewer than two stems or nothing common).
fn shared_underscore_prefix(stems: &[String]) -> String {
    if stems.len() < 2 {
        return String::new();
    }
    let first = &stems[0];
    let mut prefix = String::new();
    for (i, _) in first.match_indices('_') {
        let cand = &first[..=i]; // include the underscore
        if stems.iter().all(|s| s.starts_with(cand)) {
            prefix = cand.to_string();
        }
    }
    prefix
}

/// Resolve a (possibly partial) hint token to an actual top name from `names`,
/// case-insensitively: exact → `token + " top"` exact → unique substring. A token
/// matching several names (and no `… top`) is an error listing the candidates;
/// an unmatched token errors too — so a typo'd hint fails loudly, not silently.
fn resolve_top_name(token: &str, names: &[&str]) -> Result<String> {
    let t = token.trim();
    if let Some(n) = names.iter().find(|n| n.eq_ignore_ascii_case(t)) {
        return Ok(n.to_string());
    }
    let with_top = format!("{t} top");
    if let Some(n) = names.iter().find(|n| n.eq_ignore_ascii_case(&with_top)) {
        return Ok(n.to_string());
    }
    let lc = t.to_ascii_lowercase();
    let hits: Vec<&str> = names
        .iter()
        .copied()
        .filter(|n| n.to_ascii_lowercase().contains(&lc))
        .collect();
    match hits.as_slice() {
        [one] => Ok(one.to_string()),
        [] => Err(GeoError::Parse(format!(
            "strat hint: no top matches '{token}'"
        ))),
        many => Err(GeoError::Parse(format!(
            "strat hint: '{token}' is ambiguous — matches {}",
            many.join(", ")
        ))),
    }
}

/// Recursively collect every file under `dir` into `out`.
fn collect_files(dir: &Path, out: &mut Vec<std::path::PathBuf>) -> Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            collect_files(&path, out)?;
        } else if path.is_file() {
            out.push(path);
        }
    }
    Ok(())
}

/// Normalize a well id/filename token for matching: lower-cased, with `/`, `-`,
/// and space folded to `_` (so id `99/9-1` ↔ filename stem `99_9-1`).
fn normalize_id(s: &str) -> String {
    s.trim().to_ascii_lowercase().replace(['/', '-', ' '], "_")
}

/// Whether a file's name belongs to the well `id_key` (normalized) — its
/// normalized stem starts with the id followed by `_` or end (so `99_9-1_A`
/// matches `99/9-1` but `99_9-10` does not).
fn file_matches_id(path: &Path, id_key: &str) -> bool {
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    let norm = normalize_id(stem);
    norm == *id_key
        || norm
            .strip_prefix(id_key)
            .is_some_and(|r| r.starts_with('_'))
}

/// Load a LAS file's curves, tagging them [`LogKind::Core`] when the filename
/// marks core data (contains `core`, case-insensitive), else [`LogKind::Log`].
fn load_tagged_logs(path: &Path) -> Result<Vec<Log>> {
    let is_core = path
        .file_stem()
        .and_then(|s| s.to_str())
        .is_some_and(|s| s.to_ascii_lowercase().contains("core"));
    let logs = Log::load_las_all(path)?;
    Ok(if is_core {
        logs.into_iter()
            .map(|l| l.with_kind(LogKind::Core))
            .collect()
    } else {
        logs
    })
}

/// Whether a Petrel tops `Well` field names the loaded well `id`: an exact match,
/// or `id` followed by a separator (so `"99/9-1"` matches `"99/9-1 B"` but not
/// `"99/9-10"`).
fn well_name_matches(id: &str, record_well: &str) -> bool {
    let rec = record_well.trim();
    rec == id
        || rec
            .strip_prefix(id)
            .is_some_and(|rest| rest.starts_with([' ', '_', '-']))
}

/// The bore label in a Petrel `Well` field after the well id (e.g.
/// `("99/9-1", "99/9-1 B")` → `"B"`); empty for the main bore.
fn bore_suffix(id: &str, record_well: &str) -> String {
    record_well
        .trim()
        .strip_prefix(id)
        .unwrap_or("")
        .trim_matches([' ', '_', '-'])
        .to_string()
}

/// The bore label whose token appears in `path`'s filename (split on `_`/`-`/`.`/
/// space), or the main bore `""` if none matches.
fn route_bore(path: &Path, labels: &[String]) -> String {
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    let tokens: Vec<&str> = stem.split(['_', '-', '.', ' ']).collect();
    labels
        .iter()
        .find(|label| !label.is_empty() && tokens.iter().any(|t| t.eq_ignore_ascii_case(label)))
        .cloned()
        .unwrap_or_default()
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
