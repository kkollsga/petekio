//! Public, bounded content sniffing for the formats petekIO already knows how
//! to ingest. Detection reads only a header window and uses the filename
//! extension only as a fallback/tiebreaker.

use crate::foundation::Result;
use std::io::Read;
use std::path::Path;

const HEADER_BYTES: u64 = 64 * 1024;

/// The file formats petekIO can identify from a bounded header read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormatKind {
    Cps3Grid,
    Cps3Lines,
    IrapClassicGrid,
    IrapClassicPoints,
    EarthVisionGrid,
    Las,
    WellPath,
    PetrelTops,
    CrsMetaXml,
    GeoJson,
    CsvPoints,
    Unknown,
}

/// Detect the likely format of `path` from at most 64 KiB of leading bytes.
pub fn detect(path: impl AsRef<Path>) -> Result<FormatKind> {
    let path = path.as_ref();
    let mut file = std::fs::File::open(path)?;
    let mut bytes = Vec::new();
    file.by_ref().take(HEADER_BYTES).read_to_end(&mut bytes)?;
    let text = crate::io::decode_latin1(&bytes);
    Ok(detect_text(&text).unwrap_or_else(|| detect_extension(path)))
}

fn detect_text(text: &str) -> Option<FormatKind> {
    let mut saw_cps3 = false;
    let mut saw_cps3_grid = false;
    let mut saw_cps3_block = false;
    let mut saw_ev = false;
    let mut saw_well_head = false;
    let mut saw_wellpath_columns = false;
    let mut tops_cols = HeaderSet::default();
    let mut csv_header: Option<Vec<String>> = None;
    let mut first_data = None;

    for line in text.lines().take(160) {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        if first_data.is_none() && !t.starts_with('#') && !t.starts_with('!') {
            first_data = Some(t);
        }
        let up = t.to_ascii_uppercase();
        if t.starts_with('~') {
            return Some(FormatKind::Las);
        }
        if up.contains("PETREL WELL TOPS") {
            return Some(FormatKind::PetrelTops);
        }
        if up.contains("EARTHVISION")
            || up.contains("GRID_SIZE")
            || up.contains("GRID_SPACE")
            || (t.starts_with('#') && up.contains("FIELD:"))
        {
            saw_ev = true;
        }
        if up.contains("WELL TRACE") || up.contains("WELL HEAD") || up.contains("KELLY BUSHING") {
            saw_well_head = true;
        }
        if looks_like_wellpath_columns(&up) {
            saw_wellpath_columns = true;
        }

        let first = t
            .split_whitespace()
            .next()
            .unwrap_or("")
            .to_ascii_uppercase();
        if t.starts_with("->") {
            saw_cps3 = true;
            saw_cps3_block = true;
        } else if first.starts_with("FS") || first.starts_with("FF") {
            saw_cps3 = true;
            if matches!(first.as_str(), "FSNROW" | "FSXINC" | "FSLIMI") {
                saw_cps3_grid = true;
            }
        }
        tops_cols.observe(&up);
        if csv_header.is_none() && maybe_csv_header(t) {
            csv_header = Some(split_csv_header(t));
        }
    }

    if saw_cps3_grid {
        return Some(FormatKind::Cps3Grid);
    }
    if saw_cps3 && saw_cps3_block {
        return Some(FormatKind::Cps3Lines);
    }
    if saw_ev {
        return Some(FormatKind::EarthVisionGrid);
    }
    if saw_well_head && saw_wellpath_columns {
        return Some(FormatKind::WellPath);
    }
    if tops_cols.is_petrel_tops() {
        return Some(FormatKind::PetrelTops);
    }
    if let Some(kind) = detect_structured_prefix(text) {
        return Some(kind);
    }
    if let Some(cols) = csv_header {
        if has_xyz_columns(&cols) {
            return Some(FormatKind::CsvPoints);
        }
    }
    if let Some(line) = first_data {
        if looks_like_irap_classic(line) {
            return Some(FormatKind::IrapClassicGrid);
        }
        if looks_like_xyz_row(line) {
            return Some(FormatKind::IrapClassicPoints);
        }
    }
    None
}

fn detect_structured_prefix(text: &str) -> Option<FormatKind> {
    let t = text.trim_start();
    let lower = t.chars().take(512).collect::<String>().to_ascii_lowercase();
    if (lower.starts_with("<?xml") || lower.starts_with("<crsmeta")) && lower.contains("crsmeta") {
        return Some(FormatKind::CrsMetaXml);
    }
    if (lower.starts_with('{') || lower.starts_with('['))
        && lower.contains("\"type\"")
        && (lower.contains("featurecollection")
            || lower.contains("\"feature\"")
            || lower.contains("point")
            || lower.contains("polygon"))
    {
        return Some(FormatKind::GeoJson);
    }
    None
}

fn detect_extension(path: &Path) -> FormatKind {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "cps3grid" => FormatKind::Cps3Grid,
        "cps3lines" => FormatKind::Cps3Lines,
        "irap" | "gri" => FormatKind::IrapClassicGrid,
        "xyz" | "dat" | "irapclassicpoints" => FormatKind::IrapClassicPoints,
        "earthvisiongrid" => FormatKind::EarthVisionGrid,
        "las" => FormatKind::Las,
        "wellpath" => FormatKind::WellPath,
        "tops" => FormatKind::PetrelTops,
        "xml" => FormatKind::CrsMetaXml,
        "geojson" | "json" => FormatKind::GeoJson,
        "csv" => FormatKind::CsvPoints,
        _ => FormatKind::Unknown,
    }
}

fn looks_like_irap_classic(line: &str) -> bool {
    let mut it = line.split_whitespace();
    matches!(it.next(), Some("-996")) && it.next().is_some_and(|v| v.parse::<usize>().is_ok())
}

fn looks_like_xyz_row(line: &str) -> bool {
    let nums = line
        .split(|c: char| c.is_whitespace() || c == ',')
        .filter(|t| !t.is_empty())
        .take(4)
        .filter(|t| t.parse::<f64>().is_ok())
        .count();
    nums >= 3
}

fn looks_like_wellpath_columns(up: &str) -> bool {
    up.contains("MD") && up.contains("TVD") && up.contains("INCL") && up.contains("AZIM")
}

fn maybe_csv_header(line: &str) -> bool {
    line.contains(',') && line.chars().any(|c| c.is_ascii_alphabetic())
}

fn split_csv_header(line: &str) -> Vec<String> {
    line.split(',')
        .map(|c| c.trim().trim_matches('"').to_ascii_lowercase())
        .collect()
}

fn has_xyz_columns(cols: &[String]) -> bool {
    let has = |names: &[&str]| cols.iter().any(|c| names.iter().any(|n| c == n));
    has(&["x", "easting"]) && has(&["y", "northing"]) && has(&["z", "depth", "tvd"])
}

#[derive(Default)]
struct HeaderSet {
    md: bool,
    kind: bool,
    surface: bool,
    well: bool,
}

impl HeaderSet {
    fn observe(&mut self, up: &str) {
        match up {
            "MD" => self.md = true,
            "TYPE" => self.kind = true,
            "SURFACE" => self.surface = true,
            "WELL" => self.well = true,
            _ => {}
        }
    }

    fn is_petrel_tops(&self) -> bool {
        self.md && self.kind && self.surface && self.well
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_is_fallback_only() {
        let text = "~Version\n VERS. 2.0 :\n~Well\n";
        assert_eq!(detect_text(text), Some(FormatKind::Las));
    }
}
