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

fn section(kind: &str, name: &str, payload: Vec<u8>) -> Section {
    Section {
        kind: kind.to_string(),
        name: name.to_string(),
        tags: Vec::new(),
        version: DATA_VERSION,
        payload,
    }
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

    /// Save the whole project to a single `.pproj` file (written atomically).
    pub fn save(&self, path: impl AsRef<Path>) -> Result<()> {
        let mut sections: Vec<Section> = Vec::new();
        for (name, s) in &self.surfaces {
            sections.push(section("surface", name, serial::to_bytes(s)?));
        }
        for (id, w) in &self.wells {
            sections.push(section("well", id, serial::to_bytes(w)?));
        }
        for (name, p) in &self.points {
            sections.push(section("points", name, serial::to_bytes(p)?));
        }
        for (name, p) in &self.polygons {
            sections.push(section("polygons", name, serial::to_bytes(p)?));
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
                _ => {} // model/* or unknown → skipped (forward-compatible)
            }
        }
        Ok(geo)
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

fn from_json<T: serde::de::DeserializeOwned + Default>(app: &Value, key: &str) -> T {
    app.get(key)
        .cloned()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default()
}
