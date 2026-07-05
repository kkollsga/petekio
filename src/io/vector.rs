//! Vector IO via `geozero` — GeoJSON (points + polygons) and ESRI shapefile
//! (polygons). Wraps `geozero`'s streaming `FeatureProcessor` so that GeoJSON
//! `properties{}` are carried into PointSet attribute columns (`to_geo()` would
//! drop them). Imports only from `foundation` (+ `io::csv_points` for the
//! shared `LoadedPoints` type).

use crate::foundation::{GeoError, Result};
use crate::io::csv_points::LoadedPoints;
use geozero::error::Result as GzResult;
use geozero::{ColumnValue, CoordDimensions, FeatureProcessor, GeomProcessor, PropertyProcessor};
use indexmap::{IndexMap, IndexSet};
use std::collections::HashMap;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;

/// Coerce a `geozero` property value to `f64`, or `None` for non-numeric
/// (string/json/datetime/binary) values — those aren't attribute columns.
fn col_to_f64(v: &ColumnValue) -> Option<f64> {
    use ColumnValue::*;
    Some(match v {
        Byte(x) => *x as f64,
        UByte(x) => *x as f64,
        Bool(b) => {
            if *b {
                1.0
            } else {
                0.0
            }
        }
        Short(x) => *x as f64,
        UShort(x) => *x as f64,
        Int(x) => *x as f64,
        UInt(x) => *x as f64,
        Long(x) => *x as f64,
        ULong(x) => *x as f64,
        Float(x) => *x as f64,
        Double(x) => *x,
        _ => return None,
    })
}

/// Streaming collector for point features: one `[x, y, z]` per feature plus its
/// numeric properties. Schemaless GeoJSON is handled by accumulating the union
/// of property names (first-seen order) and NaN-filling absent cells.
#[derive(Default)]
struct PointCollector {
    coords: Vec<[f64; 3]>,
    rows: Vec<HashMap<String, f64>>,
    keys: IndexSet<String>,
    cur: Option<[f64; 3]>,
    cur_props: HashMap<String, f64>,
}

impl PointCollector {
    fn into_parts(self) -> LoadedPoints {
        let mut attrs: IndexMap<String, Vec<f64>> = IndexMap::new();
        for k in &self.keys {
            let col = self
                .rows
                .iter()
                .map(|r| r.get(k).copied().unwrap_or(f64::NAN))
                .collect();
            attrs.insert(k.clone(), col);
        }
        (self.coords, attrs)
    }
}

impl GeomProcessor for PointCollector {
    fn dimensions(&self) -> CoordDimensions {
        CoordDimensions::xyz() // request Z so `coordinate` carries it
    }
    fn xy(&mut self, x: f64, y: f64, _idx: usize) -> GzResult<()> {
        self.cur = Some([x, y, 0.0]);
        Ok(())
    }
    fn coordinate(
        &mut self,
        x: f64,
        y: f64,
        z: Option<f64>,
        _m: Option<f64>,
        _t: Option<f64>,
        _tm: Option<u64>,
        _idx: usize,
    ) -> GzResult<()> {
        self.cur = Some([x, y, z.unwrap_or(0.0)]);
        Ok(())
    }
}

impl PropertyProcessor for PointCollector {
    fn property(&mut self, _idx: usize, name: &str, value: &ColumnValue) -> GzResult<bool> {
        if let Some(f) = col_to_f64(value) {
            self.cur_props.insert(name.to_string(), f);
            self.keys.insert(name.to_string());
        }
        Ok(false) // continue with the remaining properties
    }
}

impl FeatureProcessor for PointCollector {
    fn feature_begin(&mut self, _idx: u64) -> GzResult<()> {
        self.cur = None;
        self.cur_props.clear();
        Ok(())
    }
    fn feature_end(&mut self, _idx: u64) -> GzResult<()> {
        if let Some(c) = self.cur.take() {
            self.coords.push(c);
            self.rows.push(std::mem::take(&mut self.cur_props));
        } else {
            self.cur_props.clear();
        }
        Ok(())
    }
}

/// Streaming collector for polygon/line rings — each `LineString` (polygon
/// exterior or hole, or a tagged line) becomes one ring of `[x, y, z]`.
#[derive(Default)]
struct RingCollector {
    rings: Vec<Vec<[f64; 3]>>,
    cur: Vec<[f64; 3]>,
}

impl GeomProcessor for RingCollector {
    fn xy(&mut self, x: f64, y: f64, _idx: usize) -> GzResult<()> {
        self.cur.push([x, y, 0.0]);
        Ok(())
    }
    fn linestring_begin(&mut self, _tagged: bool, _size: usize, _idx: usize) -> GzResult<()> {
        self.cur = Vec::new();
        Ok(())
    }
    fn linestring_end(&mut self, _tagged: bool, _idx: usize) -> GzResult<()> {
        if !self.cur.is_empty() {
            self.rings.push(std::mem::take(&mut self.cur));
        }
        Ok(())
    }
}

impl PropertyProcessor for RingCollector {}
impl FeatureProcessor for RingCollector {}

/// Load point features (with numeric attributes) from a GeoJSON file.
pub fn load_point_set_geojson(path: &Path) -> Result<LoadedPoints> {
    let reader = BufReader::new(File::open(path)?);
    let mut c = PointCollector::default();
    geozero::geojson::read_geojson(reader, &mut c)
        .map_err(|e| GeoError::Parse(format!("GeoJSON points: {e}")))?;
    Ok(c.into_parts())
}

/// Load polygon rings from a GeoJSON file.
pub fn load_polygon_rings_geojson(path: &Path) -> Result<Vec<Vec<[f64; 3]>>> {
    let reader = BufReader::new(File::open(path)?);
    let mut c = RingCollector::default();
    geozero::geojson::read_geojson(reader, &mut c)
        .map_err(|e| GeoError::Parse(format!("GeoJSON polygons: {e}")))?;
    Ok(c.rings)
}

/// Load polygon rings from an ESRI shapefile (the `.shp` path; `.shx`/`.dbf` are
/// picked up alongside it if present, but geometry alone is read here).
pub fn load_polygon_rings_shapefile(path: &Path) -> Result<Vec<Vec<[f64; 3]>>> {
    let reader = geozero::shp::ShpReader::from_path(path)
        .map_err(|e| GeoError::Parse(format!("shapefile: {e}")))?;
    let mut c = RingCollector::default();
    for shape in reader.iter_geometries(&mut c) {
        shape.map_err(|e| GeoError::Parse(format!("shapefile: {e}")))?;
    }
    Ok(c.rings)
}
