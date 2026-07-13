//! Whole-project persistence: a `GeoData` ↔ a single **`.pproj`** file.
//!
//! `save` assembles every element into a container section plus a JSON manifest
//! (unit / owner / created+modified / tags / strat_hints / strat_order) and
//! writes atomically. `open` reads the manifest and materializes the elements;
//! generic `asset` and opaque `model` sections are retained without interpreting
//! provider data. `inspect` reads only the manifest — list a project without
//! decoding any element.

use crate::core::persist::Persistable;
use crate::core::{PointSet, PolygonSet, StructuredMeshSurface, Surface, Well};
use crate::foundation::{GeoError, Result, Unit};
use crate::io::container::{self, Section};
use crate::io::serial::DATA_VERSION;
use crate::manager::GeoData;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

const ASSET_SECTION_KIND: &str = "asset";
const ASSET_PREFIX: &str = "@asset/";
const ASSET_FRAME_VERSION: u32 = 1;
const ASSET_MAGIC: &[u8; 8] = b"PIOASSET";

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

/// A provider-owned project asset. `envelope` is canonical UTF-8 JSON for new
/// assets; `bytes` is an opaque provider payload. petekIO validates only the
/// generic envelope fields and never interprets renderer/domain semantics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectAsset {
    pub kind: String,
    pub envelope: Vec<u8>,
    pub version: u32,
    pub tags: Vec<String>,
    pub bytes: Vec<u8>,
    // Opened assets retain the complete framed payload. This is deliberately
    // reused on save so unknown envelope fields and future versions are exact.
    raw_payload: Option<Vec<u8>>,
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
    /// (e.g. `"model/field-a/props"`); each carries its own `version`.
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

    /// Add a generic asset under a collision-safe physical name such as
    /// `@asset/templates/reservoir`. Existing names are never overwritten.
    pub fn add_asset(
        &mut self,
        name: impl Into<String>,
        kind: impl Into<String>,
        envelope: Vec<u8>,
        tags: Vec<String>,
        version: u32,
        bytes: Vec<u8>,
    ) -> Result<()> {
        let name = name.into();
        let kind = kind.into();
        validate_asset_name(&name)?;
        validate_new_asset(&kind, &envelope, version)?;
        if self.assets.contains_key(&name) {
            return Err(GeoError::Parse(format!("asset '{name}' already exists")));
        }
        self.assets.insert(
            name,
            ProjectAsset {
                kind,
                envelope,
                version,
                tags,
                bytes,
                raw_payload: None,
            },
        );
        Ok(())
    }

    /// Replace an existing generic asset. Missing names are an error.
    pub fn replace_asset(
        &mut self,
        name: &str,
        kind: impl Into<String>,
        envelope: Vec<u8>,
        tags: Vec<String>,
        version: u32,
        bytes: Vec<u8>,
    ) -> Result<()> {
        validate_asset_name(name)?;
        if !self.assets.contains_key(name) {
            return Err(GeoError::NotFound(format!("asset '{name}'")));
        }
        let kind = kind.into();
        validate_new_asset(&kind, &envelope, version)?;
        self.assets.insert(
            name.to_string(),
            ProjectAsset {
                kind,
                envelope,
                version,
                tags,
                bytes,
                raw_payload: None,
            },
        );
        Ok(())
    }

    /// Rename an asset without decoding or re-encoding it.
    pub fn rename_asset(&mut self, old: &str, new: &str) -> Result<()> {
        validate_asset_name(old)?;
        validate_asset_name(new)?;
        if old == new {
            return Ok(());
        }
        if self.assets.contains_key(new) {
            return Err(GeoError::Parse(format!("asset '{new}' already exists")));
        }
        let value = self
            .assets
            .shift_remove(old)
            .ok_or_else(|| GeoError::NotFound(format!("asset '{old}'")))?;
        self.assets.insert(new.to_string(), value);
        Ok(())
    }

    /// Delete an asset, returning whether it existed.
    pub fn delete_asset(&mut self, name: &str) -> bool {
        self.assets.shift_remove(name).is_some()
    }

    /// Collision-safe physical asset names in insertion order.
    pub fn asset_names(&self) -> Vec<String> {
        self.assets.keys().cloned().collect()
    }

    /// A snapshot of one generic asset.
    pub fn asset(&self, name: &str) -> Option<ProjectAsset> {
        self.assets.get(name).cloned()
    }

    /// Re-key a self-framed element [`Section`] to its project collection key and
    /// stamp the project's per-element tags onto it.
    fn named_section(&self, name: &str, mut sec: Section) -> Section {
        sec.name = name.to_string();
        sec.tags = self.element_tags.get(name).cloned().unwrap_or_default();
        sec
    }

    /// Save the whole project to a single `.pproj` file (written atomically).
    pub fn save(&self, path: impl AsRef<Path>) -> Result<()> {
        let mut sections: Vec<Section> = Vec::new();
        for name in self
            .surfaces
            .keys()
            .chain(self.structured_surfaces.keys())
            .chain(self.wells.keys())
            .chain(self.points.keys())
            .chain(self.polygons.keys())
            .chain(self.model_sections.keys())
        {
            if name.starts_with(ASSET_PREFIX) {
                return Err(GeoError::Parse(format!(
                    "project element name '{name}' uses reserved prefix '{ASSET_PREFIX}'"
                )));
            }
        }
        // Each element frames itself via the shared `Persistable` mapping (kind +
        // payload + version); the project overrides the section name with the
        // collection key and stamps its element tags.
        for (name, s) in &self.surfaces {
            sections.push(self.named_section(name, s.to_section()?));
        }
        for (name, s) in &self.structured_surfaces {
            sections.push(self.named_section(name, s.to_section()?));
        }
        for (id, w) in &self.wells {
            sections.push(self.named_section(id, w.to_section()?));
        }
        for (name, p) in &self.points {
            sections.push(self.named_section(name, p.to_section()?));
        }
        for (name, p) in &self.polygons {
            sections.push(self.named_section(name, p.to_section()?));
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
        for (name, asset) in &self.assets {
            validate_asset_name(name)?;
            let payload = match &asset.raw_payload {
                Some(raw) => raw.clone(),
                None => encode_asset_payload(&asset.envelope, &asset.bytes)?,
            };
            sections.push(Section {
                kind: ASSET_SECTION_KIND.to_string(),
                name: name.clone(),
                tags: asset.tags.clone(),
                version: asset.version,
                payload,
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
        Ok(container::write(
            path.as_ref(),
            &app,
            DATA_VERSION,
            &sections,
        )?)
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

        let mut physical_names = HashSet::new();
        for entry in r.entries() {
            if !physical_names.insert(entry.name.clone()) {
                return Err(GeoError::Parse(format!(
                    ".pproj contains duplicate physical section name '{}'",
                    entry.name
                )));
            }
        }

        let index: Vec<(String, String)> = r
            .entries()
            .iter()
            .map(|e| (e.kind.clone(), e.name.clone()))
            .collect();
        for (kind, name) in index {
            // Kind strings come from the same `Persistable::KIND` the writer used.
            match kind.as_str() {
                k if k == Surface::KIND => {
                    if geo.structured_surfaces.contains_key(&name) {
                        return Err(GeoError::Parse(format!(
                            ".pproj contains duplicate surface name '{name}' across surface kinds"
                        )));
                    }
                    let s = Surface::from_payload(&r.read(&name)?.payload)?;
                    geo.surfaces.insert(name, s);
                }
                k if k == StructuredMeshSurface::KIND => {
                    if geo.surfaces.contains_key(&name) {
                        return Err(GeoError::Parse(format!(
                            ".pproj contains duplicate surface name '{name}' across surface kinds"
                        )));
                    }
                    let s = StructuredMeshSurface::from_payload(&r.read(&name)?.payload)?;
                    geo.structured_surfaces.insert(name, s);
                }
                k if k == Well::KIND => {
                    let w = Well::from_payload(&r.read(&name)?.payload)?;
                    geo.wells.insert(name, w);
                }
                k if k == PointSet::KIND => {
                    let p = PointSet::from_payload(&r.read(&name)?.payload)?;
                    geo.points.insert(name, p);
                }
                k if k == PolygonSet::KIND => {
                    let p = PolygonSet::from_payload(&r.read(&name)?.payload)?;
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
                ASSET_SECTION_KIND => {
                    let s = r.read(&name)?;
                    validate_asset_name(&name)?;
                    let (kind, envelope, bytes) = if s.version == ASSET_FRAME_VERSION {
                        let (envelope, bytes) = decode_asset_payload(&s.payload)?;
                        let kind = validate_envelope(&envelope)?;
                        (kind, envelope, bytes)
                    } else {
                        // A future frame remains listable/renamable/saveable. Its
                        // provider bytes are not exposed as a current v1 asset.
                        let namespace = name[ASSET_PREFIX.len()..]
                            .split('/')
                            .next()
                            .unwrap_or("unknown")
                            .to_string();
                        (namespace, Vec::new(), Vec::new())
                    };
                    geo.assets.insert(
                        name,
                        ProjectAsset {
                            kind,
                            envelope,
                            version: s.version,
                            tags: s.tags,
                            bytes,
                            raw_payload: Some(s.payload),
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
        Ok(container::filter_to(src.as_ref(), dst.as_ref(), |e| {
            names.contains(&e.name.as_str())
                || (e.kind == ASSET_SECTION_KIND
                    && e.name.starts_with(ASSET_PREFIX)
                    && names.contains(&&e.name[ASSET_PREFIX.len()..]))
        })?)
    }

    /// Copy `src` → `dst` keeping only sections tagged with **any** of `tags` — a
    /// single shareable binary subset, byte-for-byte.
    pub fn export(src: impl AsRef<Path>, dst: impl AsRef<Path>, tags: &[&str]) -> Result<()> {
        Ok(container::filter_to(src.as_ref(), dst.as_ref(), |e| {
            e.tags.iter().any(|t| tags.contains(&t.as_str()))
        })?)
    }

    /// Merge projects `a` and `b` into `dst` (on a kind+name clash, `b` wins),
    /// copying every section byte-for-byte.
    pub fn merge(a: impl AsRef<Path>, b: impl AsRef<Path>, dst: impl AsRef<Path>) -> Result<()> {
        Ok(container::merge_to(a.as_ref(), b.as_ref(), dst.as_ref())?)
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

fn validate_asset_name(name: &str) -> Result<()> {
    if name.len() > 1024
        || !name.starts_with(ASSET_PREFIX)
        || name.contains('\\')
        || name.contains('\0')
    {
        return Err(GeoError::Parse(format!(
            "invalid asset name '{name}': expected '{ASSET_PREFIX}<collection>/<name>'"
        )));
    }
    let parts: Vec<&str> = name.split('/').collect();
    if parts.len() < 3
        || parts
            .iter()
            .any(|part| part.is_empty() || *part == "." || *part == "..")
    {
        return Err(GeoError::Parse(format!(
            "invalid asset name '{name}': empty and traversal path segments are forbidden"
        )));
    }
    Ok(())
}

fn validate_new_asset(kind: &str, envelope: &[u8], version: u32) -> Result<()> {
    if version != ASSET_FRAME_VERSION {
        return Err(GeoError::Parse(format!(
            "cannot create asset frame v{version}; supported version is {ASSET_FRAME_VERSION}"
        )));
    }
    let declared = validate_envelope(envelope)?;
    if declared != kind {
        return Err(GeoError::Parse(format!(
            "asset kind '{kind}' disagrees with envelope asset_type '{declared}'"
        )));
    }
    let value: Value = serde_json::from_slice(envelope)
        .map_err(|e| GeoError::Parse(format!("invalid asset envelope JSON: {e}")))?;
    let canonical = serde_json::to_vec(&value)
        .map_err(|e| GeoError::Parse(format!("invalid asset envelope JSON: {e}")))?;
    if canonical != envelope {
        return Err(GeoError::Parse(
            "asset envelope must be canonical compact UTF-8 JSON".into(),
        ));
    }
    Ok(())
}

fn validate_envelope(envelope: &[u8]) -> Result<String> {
    let value: Value = serde_json::from_slice(envelope)
        .map_err(|e| GeoError::Parse(format!("invalid asset envelope JSON: {e}")))?;
    let obj = value
        .as_object()
        .ok_or_else(|| GeoError::Parse("asset envelope must be a JSON object".into()))?;
    let required_string = |key: &str| -> Result<&str> {
        obj.get(key)
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| GeoError::Parse(format!("asset envelope missing non-empty '{key}'")))
    };
    let kind = required_string("asset_type")?;
    required_string("provider")?;
    required_string("codec")?;
    let schema = obj
        .get("schema_version")
        .and_then(Value::as_u64)
        .ok_or_else(|| {
            GeoError::Parse("asset envelope missing positive integer 'schema_version'".into())
        })?;
    if schema == 0 || schema > u32::MAX as u64 {
        return Err(GeoError::Parse(
            "asset envelope schema_version is out of range".into(),
        ));
    }
    Ok(kind.to_string())
}

fn encode_asset_payload(envelope: &[u8], bytes: &[u8]) -> Result<Vec<u8>> {
    let header_len = u32::try_from(envelope.len())
        .map_err(|_| GeoError::Parse("asset envelope is too large".into()))?;
    let mut out = Vec::with_capacity(12 + envelope.len() + bytes.len());
    out.extend_from_slice(ASSET_MAGIC);
    out.extend_from_slice(&header_len.to_le_bytes());
    out.extend_from_slice(envelope);
    out.extend_from_slice(bytes);
    Ok(out)
}

fn decode_asset_payload(payload: &[u8]) -> Result<(Vec<u8>, Vec<u8>)> {
    if payload.len() < 12 || &payload[..8] != ASSET_MAGIC {
        return Err(GeoError::Parse("invalid asset frame magic/length".into()));
    }
    let header_len = u32::from_le_bytes(payload[8..12].try_into().expect("four bytes")) as usize;
    let split = 12usize
        .checked_add(header_len)
        .filter(|end| *end <= payload.len())
        .ok_or_else(|| GeoError::Parse("invalid asset envelope length".into()))?;
    Ok((payload[12..split].to_vec(), payload[split..].to_vec()))
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
