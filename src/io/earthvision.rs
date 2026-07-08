//! EarthVision (Dynamic Graphics) grid ASCII reader — a **scattered `x y z`**
//! export with a directive/comment header.
//!
//! The body is one `x y z` node per line; Petrel exports often add `column` and
//! `row` as fields 4 and 5, which the point-set loader preserves as attributes.
//! The header is `#`-prefixed directive lines; the null sentinel comes
//! from a `# Null_value: <v>` directive (default `1.0e30`), and any `|z| ≥ 1e29`
//! is also treated as null. Null nodes are **dropped** (an undefined grid node
//! contributes no scattered point). Returns coordinates; `core::PointSet` builds
//! the domain object. Imports only from `foundation` (+ the io Latin-1 decoder
//! and the shared null-sentinel test).
//!
//! Handing such a file to the plain IRAP-points reader silently mis-parses the
//! header into coordinates (weakness W5); this reader — and the header sniff in
//! `io::xyz` — keep the two apart.

use crate::foundation::{GeoError, Result};
use crate::io::{is_null_sentinel, PointData, DEFAULT_NULL_1E30};
use indexmap::IndexMap;
use std::path::Path;

/// Read an EarthVision grid ASCII file into scattered `[x, y, z]` coordinates,
/// preserving optional Petrel `column`/`row` fields as point attributes when
/// the export carries them.
pub fn load_earthvision_grid(path: &Path) -> Result<PointData> {
    let text = crate::io::decode_latin1(&std::fs::read(path)?);
    let mut null = DEFAULT_NULL_1E30;
    let mut out: Vec<[f64; 3]> = Vec::new();
    let mut columns: Vec<f64> = Vec::new();
    let mut rows: Vec<f64> = Vec::new();
    let mut saw_marker = false;
    let mut saw_index_columns = false;

    for line in text.lines() {
        let s = line.trim();
        if s.is_empty() {
            continue;
        }
        if let Some(comment) = s.strip_prefix('#') {
            let up = comment.to_ascii_uppercase();
            if up.contains("EARTHVISION")
                || up.contains("GRID_SIZE")
                || up.contains("GRID_SPACE")
                || up.contains("FIELD:")
            {
                saw_marker = true;
            }
            if up.contains("NULL") {
                // `# Null_value: 1.0e30` (or `Null: …`) — take the last number.
                if let Some(v) = comment
                    .split(|c: char| c == ':' || c.is_whitespace())
                    .filter_map(|t| t.parse::<f64>().ok())
                    .next_back()
                {
                    null = v;
                }
            }
            continue;
        }
        let nums: Vec<f64> = s
            .split(|c: char| c.is_whitespace() || c == ',')
            .filter(|t| !t.is_empty())
            .filter_map(|t| t.parse::<f64>().ok())
            .collect();
        if nums.len() < 3 {
            continue;
        }
        let z = nums[2];
        if !is_null_sentinel(z, null) {
            out.push([nums[0], nums[1], z]);
            if nums.len() >= 5 {
                columns.push(nums[3]);
                rows.push(nums[4]);
                saw_index_columns = true;
            } else {
                columns.push(f64::NAN);
                rows.push(f64::NAN);
            }
        }
    }

    if !saw_marker && out.is_empty() {
        return Err(GeoError::Format(
            "EarthVision grid: no header markers and no data rows".into(),
        ));
    }
    let mut attrs = IndexMap::new();
    if saw_index_columns && columns.len() == out.len() && rows.len() == out.len() {
        attrs.insert("column".to_string(), columns);
        attrs.insert("row".to_string(), rows);
    }
    PointData::new(out, attrs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn reads_nodes_and_drops_nulls() {
        let body = "\
# Type: scattered data
# Field: 1 X
# Field: 2 Y
# Field: 3 Z meters
# Grid_size: 2 x 2
# Null_value: 1.0e30
# End:
100.0 200.0 -50.0
110.0 200.0 -51.0
100.0 210.0 1.0e30
110.0 210.0 -52.5
";
        let p = std::env::temp_dir().join("petekio_ev_grid.EarthVisionGrid");
        std::fs::File::create(&p)
            .unwrap()
            .write_all(body.as_bytes())
            .unwrap();
        let (pts, attrs) = load_earthvision_grid(&p).unwrap().into_parts();
        assert_eq!(pts.len(), 3); // the 1.0e30 node dropped
        assert_eq!(pts[0], [100.0, 200.0, -50.0]);
        assert_eq!(pts[2], [110.0, 210.0, -52.5]);
        assert!(attrs.is_empty());
    }

    #[test]
    fn preserves_petrel_column_row_fields() {
        let body = "\
# Type: scattered data
# Field: 1 x
# Field: 2 y
# Field: 3 z meters
# Field: 4 column
# Field: 5 row
# Grid_size: 2 x 2
# End:
100.0 200.0 -50.0 1 1
110.0 200.0 -51.0 2 1
100.0 210.0 -52.0 1 2
110.0 210.0 -53.0 2 2
";
        let p = std::env::temp_dir().join("petekio_ev_grid_indexed.EarthVisionGrid");
        std::fs::File::create(&p)
            .unwrap()
            .write_all(body.as_bytes())
            .unwrap();
        let (pts, attrs) = load_earthvision_grid(&p).unwrap().into_parts();
        assert_eq!(pts.len(), 4);
        assert_eq!(attrs["column"], vec![1.0, 2.0, 1.0, 2.0]);
        assert_eq!(attrs["row"], vec![1.0, 1.0, 2.0, 2.0]);
    }
}
