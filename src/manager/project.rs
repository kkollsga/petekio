//! Whole-project persistence: a `GeoData` ↔ a single **`.pproj`** file.
//!
//! `save` assembles every element into a container section plus a JSON manifest
//! (unit / owner / created+modified / tags / strat_hints / strat_order) and
//! writes atomically. `open` reads the manifest and materializes the elements;
//! unknown / `model/*` section kinds are **skipped** (forward-compatible — a
//! newer petekSim sidecar loads in an older petekIO). `inspect` reads only the
//! manifest — list a project without decoding any element.

use crate::foundation::{GeoError, Result, Unit};
use crate::io::container::{self, Section};
use crate::io::serial::{self, DATA_VERSION};
use crate::manager::GeoData;
use serde_json::{json, Value};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// An opaque petekSim model section: raw bytes + a version petekIO never parses.
#[derive(Debug, Clone)]
pub struct ModelSection {
    pub version: u32,
    pub tags: Vec<String>,
    pub bytes: Vec<u8>,
}

/// A project's manifest without its element data — the result of [`inspect`].
#[derive(Debug, Clone)]
pub struct ProjectInfo {
    pub owner: Option<String>,
    pub tags: Vec<String>,
    pub created: Option<u64>,
    pub modified: Option<u64>,
    pub unit: Option<String>,
    /// Element names grouped by kind (`surface`/`well`/`points`/`polygons`/`model/*`).
    pub elements: Vec<(String, String)>,
}

impl GeoData {
    /// The project owner recorded in the manifest, if set.
    pub fn owner(&self) -> Option<&str> {
        self.owner.as_deref()
    }
    /// Set the project owner (persisted to the manifest).
    pub fn set_owner(&mut self, owner: impl Into<String>) {
        self.owner = Some(owner.into());
    }
    /// Project-level custom tags.
    pub fn tags(&self) -> &[String] {
        &self.tags
    }
    /// Replace the project-level tags.
    pub fn set_tags(&mut self, tags: Vec<String>) {
        self.tags = tags;
    }

    /// Set a single element's custom tags (by name) — written into its section
    /// so [`export`](GeoData::export) can select it.
    pub fn set_element_tags(&mut self, name: impl Into<String>, tags: Vec<String>) {
        self.element_tags.insert(name.into(), tags);
    }

    /// Store an opaque model section (petekSim's sidecar). petekIO frames +
    /// compresses it and never parses the bytes. `name` is the section path
    /// (e.g. `"model/cerisa/props"`); each carries its own `version`.
    pub fn put_model_section(
        &mut self,
        name: impl Into<String>,
        tags: Vec<String>,
        version: u32,
        bytes: Vec<u8>,
    ) {
        self.model_sections.insert(
            name.into(),
            ModelSection {
                version,
                tags,
                bytes,
            },
        );
    }

    /// The names of the model sections currently held.
    pub fn model_section_names(&self) -> Vec<String> {
        self.model_sections.keys().cloned().collect()
    }

    /// A model section's `(version, bytes)`, or `None`. petekSim decodes these.
    pub fn model_section(&self, name: &str) -> Option<(u32, Vec<u8>)> {
        self.model_sections
            .get(name)
            .map(|m| (m.version, m.bytes.clone()))
    }

    /// Save the whole project to a single `.pproj` file (written atomically).
    pub fn save(&self, path: impl AsRef<Path>) -> Result<()> {
        let mut sections: Vec<Section> = Vec::new();
        let elem = |kind: &str, name: &str, payload: Vec<u8>| Section {
            kind: kind.to_string(),
            name: name.to_string(),
            tags: self.element_tags.get(name).cloned().unwrap_or_default(),
            version: DATA_VERSION,
            payload,
        };
        for (name, s) in &self.surfaces {
            sections.push(elem("surface", name, serial::to_bytes(s)?));
        }
        for (id, w) in &self.wells {
            sections.push(elem("well", id, serial::to_bytes(w)?));
        }
        for (name, p) in &self.points {
            sections.push(elem("points", name, serial::to_bytes(p)?));
        }
        for (name, p) in &self.polygons {
            sections.push(elem("polygons", name, serial::to_bytes(p)?));
        }
        for (name, m) in &self.model_sections {
            sections.push(Section {
                kind: "model".to_string(),
                name: name.clone(),
                tags: m.tags.clone(),
                version: m.version,
                payload: m.bytes.clone(),
            });
        }
        let now = now_secs();
        let app = json!({
            "petekio_version": env!("CARGO_PKG_VERSION"),
            "unit": self.unit,
            "owner": self.owner,
            "created": self.created.unwrap_or(now),
            "modified": now,
            "tags": self.tags,
            "strat_hints": self.strat_hints,
            "strat_order": self.strat_order,
        });
        container::write(path.as_ref(), &app, DATA_VERSION, &sections)
    }

    /// Open a `.pproj` project. Surfaces/wells/points/polygons are materialized;
    /// `model/*` (petekSim's opaque sidecar) and any unknown section kind are
    /// skipped so a newer sidecar still loads in an older petekIO.
    pub fn open(path: impl AsRef<Path>) -> Result<GeoData> {
        let mut r = container::open(path.as_ref())?;
        migrate_gate(r.data_version())?;
        let app = r.app().clone();
        let unit: Unit = app
            .get("unit")
            .cloned()
            .and_then(|v| serde_json::from_value(v).ok())
            .ok_or_else(|| GeoError::Parse(".pproj manifest missing/invalid unit".into()))?;
        let mut geo = GeoData::new(unit);
        geo.owner = app.get("owner").and_then(|v| v.as_str()).map(String::from);
        geo.tags = from_json(&app, "tags");
        geo.created = app.get("created").and_then(Value::as_u64);
        geo.strat_hints = from_json(&app, "strat_hints");
        geo.strat_order = from_json(&app, "strat_order");

        let index: Vec<(String, String)> = r
            .entries()
            .iter()
            .map(|e| (e.kind.clone(), e.name.clone()))
            .collect();
        for (kind, name) in index {
            match kind.as_str() {
                "surface" => {
                    let s = serial::from_bytes(&r.read(&name)?.payload)?;
                    geo.surfaces.insert(name, s);
                }
                "well" => {
                    let w = serial::from_bytes(&r.read(&name)?.payload)?;
                    geo.wells.insert(name, w);
                }
                "points" => {
                    let p = serial::from_bytes(&r.read(&name)?.payload)?;
                    geo.points.insert(name, p);
                }
                "polygons" => {
                    let p = serial::from_bytes(&r.read(&name)?.payload)?;
                    geo.polygons.insert(name, p);
                }
                "model" => {
                    // Opaque — held as raw bytes, never parsed.
                    let s = r.read(&name)?;
                    geo.model_sections.insert(
                        name,
                        ModelSection {
                            version: s.version,
                            tags: s.tags,
                            bytes: s.payload,
                        },
                    );
                }
                _ => {} // unknown kind → skipped (forward-compatible)
            }
        }
        Ok(geo)
    }

    /// Copy `src` → `dst` keeping only sections whose name is in `names`
    /// (byte-for-byte — model sections included, never re-encoded).
    pub fn split(src: impl AsRef<Path>, dst: impl AsRef<Path>, names: &[&str]) -> Result<()> {
        container::filter_to(src.as_ref(), dst.as_ref(), |e| {
            names.contains(&e.name.as_str())
        })
    }

    /// Copy `src` → `dst` keeping only sections tagged with **any** of `tags` — a
    /// single shareable binary subset, byte-for-byte.
    pub fn export(src: impl AsRef<Path>, dst: impl AsRef<Path>, tags: &[&str]) -> Result<()> {
        container::filter_to(src.as_ref(), dst.as_ref(), |e| {
            e.tags.iter().any(|t| tags.contains(&t.as_str()))
        })
    }

    /// Merge projects `a` and `b` into `dst` (on a kind+name clash, `b` wins),
    /// copying every section byte-for-byte.
    pub fn merge(a: impl AsRef<Path>, b: impl AsRef<Path>, dst: impl AsRef<Path>) -> Result<()> {
        container::merge_to(a.as_ref(), b.as_ref(), dst.as_ref())
    }

    /// Read a project's manifest without decoding any element (partial open).
    pub fn inspect(path: impl AsRef<Path>) -> Result<ProjectInfo> {
        let r = container::open(path.as_ref())?;
        let app = r.app();
        Ok(ProjectInfo {
            owner: app.get("owner").and_then(|v| v.as_str()).map(String::from),
            tags: from_json(app, "tags"),
            created: app.get("created").and_then(Value::as_u64),
            modified: app.get("modified").and_then(Value::as_u64),
            unit: app.get("unit").and_then(|v| v.as_str()).map(String::from),
            elements: r
                .entries()
                .iter()
                .map(|e| (e.kind.clone(), e.name.clone()))
                .collect(),
        })
    }
}

/// The element-schema (`data_version`) compatibility gate. A file newer than
/// this build is refused with a clear message; an older one is routed through a
/// migration (none needed at v1 — the hook is here so a future bump lands a
/// `data_version < N` decoder instead of a hard break).
fn migrate_gate(file_version: u32) -> Result<()> {
    if file_version > DATA_VERSION {
        return Err(GeoError::Parse(format!(
            ".pproj element schema v{file_version} is newer than this petekIO (reads ≤ v{DATA_VERSION}) — upgrade petekIO"
        )));
    }
    // file_version < DATA_VERSION → migrate here (per-version decoders). At v1
    // there is nothing older, so current-version files pass straight through.
    Ok(())
}

fn from_json<T: serde::de::DeserializeOwned + Default>(app: &Value, key: &str) -> T {
    app.get(key)
        .cloned()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::foundation::Unit;
    use serde_json::json;

    fn tmp(tag: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("pio_ver_{tag}_{}.pproj", std::process::id()))
    }

    #[test]
    fn rejects_newer_element_schema() {
        let p = tmp("newer");
        container::write(&p, &json!({"unit": "Metres"}), DATA_VERSION + 1, &[]).unwrap();
        let err = GeoData::open(&p)
            .err()
            .expect("newer schema must be rejected");
        assert!(format!("{err}").contains("newer than this petekIO"));
        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn current_version_opens_and_restores_unit() {
        let p = tmp("cur");
        container::write(&p, &json!({"unit": "Feet"}), DATA_VERSION, &[]).unwrap();
        assert_eq!(GeoData::open(&p).unwrap().unit, Unit::Feet);
        std::fs::remove_file(&p).ok();
    }
}
