//! Human-readable export writers for scattered points and polygon rings —
//! GeoJSON + CSV. The write side of `io::vector` / `io::csv_points`, so an
//! element round-trips through its interchange format. `NaN` → `null` (GeoJSON)
//! or the literal `NaN` (CSV), both of which parse back to `f64::NAN`.

use crate::foundation::{GeoError, Result};
use indexmap::IndexMap;
use serde_json::{json, Map, Value};
use std::io::Write;
use std::path::Path;

fn num(v: f64) -> Value {
    if v.is_finite() {
        json!(v)
    } else {
        Value::Null // NaN/Inf aren't representable in JSON
    }
}

/// Write scattered points (`[x, y, z]` + attribute columns) as a GeoJSON
/// `FeatureCollection` of `Point`s (attributes become feature properties).
pub fn write_points_geojson(
    path: &Path,
    coords: &[[f64; 3]],
    attrs: &IndexMap<String, Vec<f64>>,
) -> Result<()> {
    let features: Vec<Value> = coords
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let mut props = Map::new();
            for (k, col) in attrs {
                props.insert(k.clone(), num(col.get(i).copied().unwrap_or(f64::NAN)));
            }
            json!({
                "type": "Feature",
                "geometry": {"type": "Point", "coordinates": [c[0], c[1], c[2]]},
                "properties": Value::Object(props),
            })
        })
        .collect();
    let fc = json!({"type": "FeatureCollection", "features": features});
    write_json(path, &fc)
}

/// Write polygon `rings` (each `[x, y, z]`) as a GeoJSON `FeatureCollection` of
/// `Polygon`s (2-D exteriors; z is dropped, matching the areal model).
pub fn write_polygons_geojson(path: &Path, rings: &[Vec<[f64; 3]>]) -> Result<()> {
    let features: Vec<Value> = rings
        .iter()
        .map(|ring| {
            let ext: Vec<Value> = ring.iter().map(|p| json!([p[0], p[1]])).collect();
            json!({
                "type": "Feature",
                "geometry": {"type": "Polygon", "coordinates": [ext]},
                "properties": {},
            })
        })
        .collect();
    let fc = json!({"type": "FeatureCollection", "features": features});
    write_json(path, &fc)
}

/// Write scattered points as CSV with `x,y,z` + one column per attribute.
pub fn write_points_csv(
    path: &Path,
    coords: &[[f64; 3]],
    attrs: &IndexMap<String, Vec<f64>>,
) -> Result<()> {
    let mut out = String::from("x,y,z");
    for k in attrs.keys() {
        out.push(',');
        out.push_str(k);
    }
    out.push('\n');
    for (i, c) in coords.iter().enumerate() {
        out.push_str(&format!("{},{},{}", c[0], c[1], c[2]));
        for col in attrs.values() {
            out.push(',');
            out.push_str(&col.get(i).copied().unwrap_or(f64::NAN).to_string());
        }
        out.push('\n');
    }
    let mut f = std::fs::File::create(path)?;
    f.write_all(out.as_bytes())?;
    Ok(())
}

fn write_json(path: &Path, value: &Value) -> Result<()> {
    let s = serde_json::to_string(value).map_err(|e| GeoError::Parse(e.to_string()))?;
    std::fs::File::create(path)?.write_all(s.as_bytes())?;
    Ok(())
}
