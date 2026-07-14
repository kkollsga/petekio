//! `GeoData` ingest: the extension-dispatched `load_*` methods and the private
//! well-routing helpers (id normalization, bore labelling/routing, tops-record
//! matching, recursive file collection). Split out of `geodata.rs` so the
//! substrate module stays about the project state, and the file-routing logic
//! gets its own compartment.
//!
//! The loaders dispatch on file extension over the formats the `io`/`core`
//! layers already support; unknown extensions are a typed `GeoError`. See each
//! method for the formats it accepts.

use crate::core::{
    FluidContact, Log, LogKind, PointSet, PolygonSet, Station, StructuredMeshSurface, Surface, Top,
    TrajectoryInput, Well,
};
use crate::foundation::{GeoError, Point3, Result};
use crate::manager::GeoData;
use crate::FormatKind;
use indexmap::IndexMap;
use std::path::{Path, PathBuf};

/// Lower-cased file extension of `path`, or `""` when it has none.
fn ext_of(path: &Path) -> String {
    path.extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
}

#[derive(Debug, Clone)]
struct ClassifiedFile {
    path: PathBuf,
    kind: FormatKind,
}

fn classify(path: &Path) -> Result<FormatKind> {
    crate::io::detect::detect(path)
}

impl GeoData {
    /// Load a surface from `path` and store it under `name`, dispatching
    /// content-first (`detect(path)`), with extension fallback only when the
    /// detector returns `Unknown`: IRAP classic (FIRST) ASCII grid or CPS-3
    /// regular grid. Returns a borrow of the stored surface.
    pub fn load_surface(&mut self, name: &str, path: impl AsRef<Path>) -> Result<&Surface> {
        if self.structured_surfaces.contains_key(name) || self.tri_surfaces.contains_key(name) {
            return Err(GeoError::Parse(format!(
                "load_surface: surface name '{name}' already belongs to a structured mesh surface"
            )));
        }
        let path = path.as_ref();
        let surface = match classify(path)? {
            FormatKind::IrapClassicGrid => Surface::load_irap_classic(path)?,
            FormatKind::Cps3Grid => Surface::load_cps3_grid(path)?,
            FormatKind::Unknown => match ext_of(path).as_str() {
                "irap" | "gri" | "" => Surface::load_irap_classic(path)?,
                "cps3grid" => Surface::load_cps3_grid(path)?,
                other => {
                    return Err(GeoError::Parse(format!(
                        "load_surface: unsupported surface extension '.{other}' for '{}'",
                        path.display()
                    )))
                }
            },
            other => {
                return Err(GeoError::Format(format!(
                    "load_surface: '{}' is {other:?}, not a supported surface format",
                    path.display()
                )))
            }
        };
        if !self.surfaces.contains_key(name) {
            self.surfaces.insert(name.to_string(), surface);
            self.surface_order.push(name.to_string());
        }
        Ok(self.surfaces.get(name).expect("just inserted or existed"))
    }

    /// Load an EarthVision explicit-node grid and store it under the shared
    /// surface namespace as a [`StructuredMeshSurface`].
    pub fn load_structured_surface(
        &mut self,
        name: &str,
        path: impl AsRef<Path>,
    ) -> Result<&StructuredMeshSurface> {
        if self.surfaces.contains_key(name) || self.tri_surfaces.contains_key(name) {
            return Err(GeoError::Parse(format!(
                "load_structured_surface: surface name '{name}' already belongs to a regular surface"
            )));
        }
        let path = path.as_ref();
        match classify(path)? {
            FormatKind::EarthVisionGrid => {}
            FormatKind::Unknown if ext_of(path) == "earthvisiongrid" => {}
            other => {
                return Err(GeoError::Format(format!(
                    "load_structured_surface: '{}' is {other:?}, not an EarthVision grid",
                    path.display()
                )))
            }
        }
        let surface = StructuredMeshSurface::load_earthvision_grid(path)?;
        if !self.structured_surfaces.contains_key(name) {
            self.structured_surfaces.insert(name.to_string(), surface);
            self.surface_order.push(name.to_string());
        }
        Ok(self
            .structured_surfaces
            .get(name)
            .expect("just inserted or existed"))
    }

    /// Load a well from `files` and store it under `id`, returning a borrow.
    ///
    /// `files` is a directory or a single file. A directory is walked
    /// **recursively** (so a Petrel export tree with separate `Paths/`/`Logs/`
    /// subdirs works, not just a flat folder); when filenames carry the well id
    /// (`99_9-1_A.wellpath`), only this well's files are taken (others sharing the
    /// tree are skipped). Each file is ingested by extension:
    /// - `*.wellpath` → a **positioned** trajectory. A **single** wellpath is the
    ///   well's one bore — the main bore `""` — so its logs/tops co-locate with
    ///   that trajectory and position through it (a deviated single-sidetrack
    ///   well then Just Works). **Multiple** wellpaths → one named bore each,
    ///   labelled by the filename stem minus the shared prefix (`99_9-1_A`/
    ///   `99_9-1_ST2` → bores `A`/`ST2`). The header's wellhead XY / KB / CRS are
    ///   taken as authoritative.
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
        // Opt-in curve canonicalization (Task 1d / weakness W9): captured before
        // the mutable borrow of `self.wells`, applied to each loaded log below.
        let aliases = self.curve_aliases.clone();

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
        let mut files: Vec<ClassifiedFile> = paths
            .into_iter()
            .map(|path| classify(&path).map(|kind| ClassifiedFile { path, kind }))
            .collect::<Result<_>>()?;

        // In a shared tree the files are well-id-named (`99_9-1_A.wellpath`);
        // keep only this well's. If no filename carries the id (a flat folder
        // with generic names like `sample.las`), every file belongs to the well.
        let id_key = normalize_id(id);
        // Compute the id match once per path (was an `any` scan + a `retain` scan,
        // each re-normalizing every stem). If any file carries the id, keep only
        // the matching ones.
        let matches: Vec<bool> = files
            .iter()
            .map(|f| file_matches_id(&f.path, &id_key))
            .collect();
        if matches.iter().any(|&m| m) {
            let mut keep = matches.iter();
            files.retain(|f| *keep.next().unwrap() || f.kind == FormatKind::CrsMetaXml);
        }

        let wellpaths: Vec<_> = files
            .iter()
            .filter(|f| f.kind == FormatKind::WellPath)
            .map(|f| f.path.clone())
            .collect();
        let las: Vec<_> = files
            .iter()
            .filter(|f| f.kind == FormatKind::Las)
            .map(|f| f.path.clone())
            .collect();
        let mut tops: Vec<Top> = Vec::new();
        for file in &files {
            if file.kind == FormatKind::CsvPoints && ext_of(&file.path) == "csv" {
                tops.extend(Top::load_csv(&file.path, "name", "md")?);
            }
        }
        let crs = files
            .iter()
            .filter(|f| f.kind == FormatKind::CrsMetaXml)
            .find_map(|f| crate::io::crsmeta::load_label(&f.path).ok());

        let mut well = Well::new(id, head, kb);
        if let Some(label) = crs {
            well.set_crs(label);
        }

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
                st.add_log(canonicalize_log(log, aliases.as_ref()));
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
                    st.add_log(canonicalize_log(log, aliases.as_ref()));
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
    /// the main bore). `Type == Horizon` picks become formation tops;
    /// `Type == Other` picks become fluid contacts (OWC/GOC/FWL, etc.). Unknown
    /// wells are skipped. Returns the number of tops assigned. (Load wells
    /// *before* tops.)
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

        // Distribute Horizon picks to tops, and Other picks to contacts.
        let ids: Vec<String> = self.wells.keys().cloned().collect();
        let mut added = 0;
        for r in recs {
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
            let st = well.sidetrack_mut(&label);
            if r.kind.eq_ignore_ascii_case("Horizon") {
                st.add_tops(vec![Top::new(r.surface, r.md)]);
                added += 1;
            } else if r.kind.eq_ignore_ascii_case("Other") {
                st.add_contacts(vec![FluidContact::new(r.surface, r.md)]);
            }
        }

        // Push the column into every loaded well, then record it on the project.
        for well in self.wells.values_mut() {
            well.set_strat_order(&order);
        }
        self.strat_order = order;
        Ok(added)
    }

    /// Load a point set from `path` and store it under `name`, returning a
    /// borrow. Dispatches content-first (`detect(path)`), with extension fallback
    /// only when the detector returns `Unknown`: GeoJSON, headered CSV with
    /// `x`/`y`/`z`, EarthVision grid ASCII, or RMS plain `X Y Z`.
    pub fn load_points(&mut self, name: &str, path: impl AsRef<Path>) -> Result<&PointSet> {
        let path = path.as_ref();
        let points = match classify(path)? {
            FormatKind::GeoJson => PointSet::load_geojson(path)?,
            FormatKind::CsvPoints => PointSet::load_csv(path, "x", "y", "z")?,
            FormatKind::EarthVisionGrid => PointSet::load_earthvision_grid(path)?,
            FormatKind::IrapClassicPoints => PointSet::load_irap_points(path)?,
            FormatKind::Unknown => match ext_of(path).as_str() {
                "geojson" | "json" => PointSet::load_geojson(path)?,
                "csv" => PointSet::load_csv(path, "x", "y", "z")?,
                "earthvisiongrid" => PointSet::load_earthvision_grid(path)?,
                "xyz" | "irap" | "dat" | "irapclassicpoints" | "" => {
                    PointSet::load_irap_points(path)?
                }
                other => {
                    return Err(GeoError::Parse(format!(
                        "load_points: unsupported point extension '.{other}' for '{}'",
                        path.display()
                    )))
                }
            },
            other => {
                return Err(GeoError::Format(format!(
                    "load_points: '{}' is {other:?}, not a supported point format",
                    path.display()
                )))
            }
        };
        let entry = self.points.entry(name.to_string());
        Ok(entry.or_insert(points))
    }

    /// Load an IRAP/RMS plain `X Y Z` point set and enrich it with Petrel
    /// `column`/`row` topology from a matching EarthVision grid export.
    pub fn load_points_with_topology(
        &mut self,
        name: &str,
        path: impl AsRef<Path>,
        topology_path: impl AsRef<Path>,
    ) -> Result<&PointSet> {
        let points = PointSet::load_irap_points_with_topology(path, topology_path)?;
        let entry = self.points.entry(name.to_string());
        Ok(entry.or_insert(points))
    }

    /// Load a polygon set from `path` and store it under `name`, returning a
    /// borrow. Dispatches content-first (`detect(path)`), with extension fallback
    /// only when the detector returns `Unknown`: GeoJSON, shapefile, CPS-3
    /// polyline blocks, or RMS rings (`999.0` separators).
    pub fn load_polygons(&mut self, name: &str, path: impl AsRef<Path>) -> Result<&PolygonSet> {
        let path = path.as_ref();
        let polygons = match classify(path)? {
            FormatKind::GeoJson => PolygonSet::load_geojson(path)?,
            FormatKind::Cps3Lines => PolygonSet::load_cps3_lines(path)?,
            FormatKind::IrapClassicPoints => PolygonSet::load_irap_polygons(path)?,
            FormatKind::Unknown => match ext_of(path).as_str() {
                "geojson" | "json" => PolygonSet::load_geojson(path)?,
                "shp" => PolygonSet::load_shapefile(path)?,
                "cps3lines" => PolygonSet::load_cps3_lines(path)?,
                "pol" | "xyz" | "irap" | "" => PolygonSet::load_irap_polygons(path)?,
                other => {
                    return Err(GeoError::Parse(format!(
                        "load_polygons: unsupported polygon extension '.{other}' for '{}'",
                        path.display()
                    )))
                }
            },
            other => {
                return Err(GeoError::Format(format!(
                    "load_polygons: '{}' is {other:?}, not a supported polygon format",
                    path.display()
                )))
            }
        };
        let entry = self.polygons.entry(name.to_string());
        Ok(entry.or_insert(polygons))
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
/// `A`/`ST2`). A **single** wellpath, or no shared prefix, → the main bore `""`:
/// with one trajectory the well is single-bore, so its logs/tops (which default
/// to the main bore) co-locate with that one path and position through it (the
/// single-trajectory rule; see [`Well::primary`](crate::core::Well)). This is
/// what lets a deviated single-sidetrack NCS well (one `.wellpath`, one comp-log)
/// position its curves without explicit bore selection.
fn bore_labels(wellpaths: &[std::path::PathBuf]) -> Vec<(std::path::PathBuf, String)> {
    // One wellpath → the single (main) bore, regardless of its stem.
    if wellpaths.len() < 2 {
        return wellpaths
            .iter()
            .map(|p| (p.clone(), String::new()))
            .collect();
    }
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
            "strat hint entry '{token}': no loaded well top matches it (from the \
             IngestSpec strat_hints / strat_hint hints)"
        ))),
        many => Err(GeoError::Parse(format!(
            "strat hint entry '{token}' is ambiguous — matches {} (disambiguate the \
             IngestSpec strat_hints / strat_hint entry)",
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

/// Canonicalize a log's mnemonic when the project has an opt-in alias map: the
/// user map first (raw vendor mnemonic → canonical), then the built-in table +
/// vintage `_YYYY` strip. A `None` map leaves the raw mnemonic untouched (the
/// default), so `PHIE_2025` stays `PHIE_2025` unless canonicalization is enabled.
fn canonicalize_log(mut log: Log, aliases: Option<&crate::analysis::NameMap>) -> Log {
    if let Some(map) = aliases {
        log.mnemonic = crate::analysis::normalize::canonical_mnemonic_with(&log.mnemonic, map);
    }
    log
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

/// Whether a Petrel tops `Well` field names the loaded well `id`, **tolerant of
/// the family's naming variants**: separators (`/`, `-`, space) and case are
/// folded to a canonical key (see [`normalize_id`]) before comparing, so the id
/// `"99/9-1 A"` matches a tops `Well` written `"99_9-1_A"`. Matches an exact
/// (normalized) name, or `id` followed by a bore suffix at a separator boundary
/// (so `"99/9-1"` matches `"99/9-1 B"` but not `"99/9-10"`).
fn well_name_matches(id: &str, record_well: &str) -> bool {
    let nid = normalize_id(id);
    let nrec = normalize_id(record_well);
    nrec == nid
        || nrec
            .strip_prefix(&nid)
            .is_some_and(|rest| rest.starts_with('_'))
}

/// The bore label in a Petrel `Well` field after the well id (e.g.
/// `("99/9-1", "99/9-1 B")` → `"B"`); empty for the main bore. The id prefix is
/// matched **variant-tolerantly** (separator/case, via [`normalize_id`]), but the
/// suffix is returned in the record's **original case** so it can key a
/// case-sensitive sidetrack label. `normalize_id` is length-preserving on a
/// trimmed ASCII well name, so the suffix begins at byte `nid.len()`.
fn bore_suffix(id: &str, record_well: &str) -> String {
    let rec = record_well.trim();
    let nid = normalize_id(id);
    if normalize_id(rec).starts_with(&nid) && rec.is_char_boundary(nid.len()) {
        rec[nid.len()..].trim_matches([' ', '_', '-']).to_string()
    } else {
        String::new()
    }
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
