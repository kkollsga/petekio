//! CPS-3 ASCII readers — the regular **grid** and the **polyline/polygon** form.
//!
//! Both are Petrel/GeoGraphix exports. This module returns primitives (a
//! geometry + value grid, or rings of `[x, y, z]`); `core::Surface` /
//! `core::PolygonSet` build the domain objects. Imports only from `foundation`.
//!
//! **CPS-3 grid** (`.CPS3grid`): an `FS*` header block then a `->` marker line,
//! followed by whitespace-separated z values in **row-major** order. Header
//! records used:
//! - `FSLIMI xmin xmax ymin ymax [zmin zmax]`
//! - `FSNROW nrow ncol` — `nrow` = Y node count, `ncol` = X node count
//! - `FSXINC xinc yinc` — node spacing (falls back to the FSLIMI extent / (n-1))
//! - `FSASCI … <null>` — the undefined sentinel is the record's last field
//!   (default `1.0E+30`); any `|z| ≥ 1e29` also maps to `NaN`.
//!
//! **Node-ordering convention (documented, deterministic):** the z stream is
//! **row-major** — each block of `ncol` values is one data row (constant Y),
//! `ncol` columns running west→east from `xmin`. Data row `r` (`0..nrow`) runs
//! **south→north**: row 0 is the `ymin` (SOUTH) edge and each next row steps
//! *up* by `yinc`. This matches the IRAP-classic baseline (`irap.rs`: the first
//! stored value is the origin node at `ymin`) and the Golden Software CPS-3
//! definition, whose values run bottom-up; CPS-3 dialects differ on row-major
//! (Petrel) vs column-major (Surfer) *ordering*, but both take the SOUTH edge as
//! the origin. It maps onto [`GridGeometry`] as `xori = xmin`, `yori = ymin`,
//! `yflip = false` (node `j` = data row `j`, node `i` = data column `i`).
//!
//! Reference: Golden Software, *CPS-3 Grid File Format* (surferhelp). No CPS-3
//! header field encodes the Y direction, so no dialect flag can be honoured here;
//! the south-origin convention above is assumed. **History:** through v0.2.8 this
//! reader used `yori = ymax, yflip = true` (north-origin), which ingested a
//! CPS-3 grid Y-**flipped** relative to the IRAP copy of the same surface.
//!
//! **CPS-3 lines** (`.CPS3lines`): an `FF*`/`FS*` header, then polyline blocks
//! each introduced by a `->` marker line, followed by that block's `x y z`
//! vertices. Each block becomes one ring.

use crate::foundation::{GeoError, GridGeometry, Result};
use crate::io::{is_null_sentinel, DEFAULT_NULL_1E30};
use ndarray::Array2;
use std::path::Path;

/// Read a CPS-3 regular grid into a geometry + value grid (nulls → `NaN`).
pub fn load_cps3_grid(path: &Path) -> Result<(GridGeometry, Array2<f64>)> {
    let text = crate::io::decode_latin1(&std::fs::read(path)?);
    parse_cps3_grid(&text)
}

fn header_nums(line: &str) -> Vec<f64> {
    line.split_whitespace()
        .skip(1) // the FS* tag
        .filter_map(|t| t.parse::<f64>().ok())
        .collect()
}

fn parse_cps3_grid(text: &str) -> Result<(GridGeometry, Array2<f64>)> {
    let (mut xmin, mut xmax, mut ymin, mut ymax) = (None, None, None, None);
    let (mut nrow, mut ncol): (Option<usize>, Option<usize>) = (None, None);
    let (mut xinc, mut yinc): (Option<f64>, Option<f64>) = (None, None);
    let mut null = DEFAULT_NULL_1E30;
    let mut values: Vec<f64> = Vec::new();
    let mut in_data = false;

    for line in text.lines() {
        let s = line.trim();
        if s.is_empty() {
            continue;
        }
        if s.starts_with("->") {
            in_data = true;
            continue;
        }
        if !in_data && (s.starts_with("FS") || s.starts_with("FF")) {
            let tag = s.split_whitespace().next().unwrap_or("");
            let nums = header_nums(s);
            match tag {
                "FSLIMI" if nums.len() >= 4 => {
                    xmin = Some(nums[0]);
                    xmax = Some(nums[1]);
                    ymin = Some(nums[2]);
                    ymax = Some(nums[3]);
                }
                "FSNROW" if nums.len() >= 2 => {
                    nrow = Some(nums[0] as usize);
                    ncol = Some(nums[1] as usize);
                }
                "FSXINC" if nums.len() >= 2 => {
                    xinc = Some(nums[0]);
                    yinc = Some(nums[1]);
                }
                "FSASCI" => {
                    if let Some(&last) = nums.last() {
                        null = last;
                    }
                }
                _ => {}
            }
            continue;
        }
        if in_data {
            values.extend(s.split_whitespace().filter_map(|t| t.parse::<f64>().ok()));
        }
    }

    let nrow = nrow.ok_or_else(|| GeoError::Parse("CPS-3 grid: missing FSNROW".into()))?;
    let ncol = ncol.ok_or_else(|| GeoError::Parse("CPS-3 grid: missing FSNROW ncol".into()))?;
    let xmin = xmin.ok_or_else(|| GeoError::Parse("CPS-3 grid: missing FSLIMI".into()))?;
    let xmax = xmax.unwrap();
    let ymin = ymin.unwrap();
    let ymax = ymax.unwrap();
    if nrow == 0 || ncol == 0 {
        return Err(GeoError::Parse("CPS-3 grid: zero node count".into()));
    }
    // Node spacing: prefer FSXINC, else derive from the extent.
    let xinc = xinc.filter(|v| *v > 0.0).unwrap_or_else(|| {
        if ncol > 1 {
            (xmax - xmin) / (ncol - 1) as f64
        } else {
            1.0
        }
    });
    let yinc = yinc.filter(|v| *v > 0.0).unwrap_or_else(|| {
        if nrow > 1 {
            (ymax - ymin) / (nrow - 1) as f64
        } else {
            1.0
        }
    });

    let n = nrow
        .checked_mul(ncol)
        .ok_or_else(|| GeoError::Parse("CPS-3 grid: nrow*ncol overflows".into()))?;
    // Tolerate a short/long stream (trailing pad): truncate or NaN-fill.
    values.resize(n, null);

    // Row-major z[r][c] → Surface value[[i=c, j=r]]; row 0 = south (ymin), so
    // data row `r` lands on node row `j = r` with `yori = ymin, yflip = false`.
    let mut grid = Array2::from_elem((ncol, nrow), f64::NAN);
    for (idx, &v) in values.iter().enumerate() {
        let r = idx / ncol; // Y row
        let c = idx % ncol; // X col
        grid[[c, r]] = if is_null_sentinel(v, null) {
            f64::NAN
        } else {
            v
        };
    }

    let geom = GridGeometry {
        xori: xmin,
        yori: ymin,
        xinc,
        yinc,
        ncol,
        nrow,
        rotation_deg: 0.0,
        yflip: false,
    };
    Ok((geom, grid))
}

/// Read CPS-3 polyline blocks into rings of `[x, y, z]`. Each `->` marker starts
/// a new block; its subsequent `x y z` lines are the ring vertices. Header
/// (`FF*`/`FS*`) lines and any line before the first `->` are ignored.
pub fn load_cps3_lines(path: &Path) -> Result<Vec<Vec<[f64; 3]>>> {
    let text = crate::io::decode_latin1(&std::fs::read(path)?);
    let mut rings: Vec<Vec<[f64; 3]>> = Vec::new();
    let mut cur: Option<Vec<[f64; 3]>> = None;
    for line in text.lines() {
        let s = line.trim();
        if s.is_empty() {
            continue;
        }
        if s.starts_with("->") {
            if let Some(r) = cur.take() {
                rings.push(r);
            }
            cur = Some(Vec::new());
            continue;
        }
        // Header records / commentary before the first block.
        if s.starts_with("FF") || s.starts_with("FS") || s.starts_with('#') {
            continue;
        }
        let Some(ring) = cur.as_mut() else { continue };
        let nums: Vec<f64> = s
            .split_whitespace()
            .filter_map(|t| t.parse().ok())
            .collect();
        if nums.len() >= 3 {
            ring.push([nums[0], nums[1], nums[2]]);
        }
    }
    if let Some(r) = cur {
        rings.push(r);
    }
    Ok(rings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write(name: &str, body: &str) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(name);
        std::fs::File::create(&p)
            .unwrap()
            .write_all(body.as_bytes())
            .unwrap();
        p
    }

    #[test]
    fn grid_maps_row_major_south_to_north() {
        // 2 cols (X) × 3 rows (Y); xmin=100 xmax=110 ymin=200 ymax=220; inc 10.
        // Row 0 (south, y=200): 1 2 ; row 1 (y=210): 3 <null> ; row 2 (north,
        // y=220): 5 6 — first data row is the SOUTH edge (ymin).
        let body = "\
FSASCI 0 1 0 5 1.0E+30
FSLIMI 100 110 200 220 0 6
FSNROW 3 2
FSXINC 10 10
->
1 2
3 1.0E+30
5 6
";
        let p = write("petekio_cps3_grid.CPS3grid", body);
        let (geom, v) = load_cps3_grid(&p).unwrap();
        assert_eq!((geom.ncol, geom.nrow), (2, 3));
        assert!(!geom.yflip);
        // node (i=col, j=row): value[[i,j]] = z[row=j][col=i]
        assert_eq!(v[[0, 0]], 1.0); // col0,row0
        assert_eq!(v[[1, 0]], 2.0); // col1,row0
        assert_eq!(v[[0, 1]], 3.0);
        assert!(v[[1, 1]].is_nan()); // null
        assert_eq!(v[[0, 2]], 5.0);
        assert_eq!(v[[1, 2]], 6.0);
        // Geometry: node (0,0) = (xmin, ymin) south-west; row 1 steps north.
        assert_eq!(geom.node_xy(0, 0), (100.0, 200.0));
        assert_eq!(geom.node_xy(1, 0), (110.0, 200.0));
        assert_eq!(geom.node_xy(0, 1), (100.0, 210.0));
        assert_eq!(geom.node_xy(0, 2), (100.0, 220.0));
    }

    #[test]
    fn lines_split_blocks_on_arrow() {
        let body = "\
FFASCI 0 1 2 3
FFATTR ...
-> 1
100.0 200.0 -50.0
110.0 200.0 -51.0
110.0 210.0 -52.0
-> 2
300.0 400.0 -60.0
310.0 400.0 -61.0
310.0 410.0 -62.0
";
        let p = write("petekio_cps3_lines.CPS3lines", body);
        let rings = load_cps3_lines(&p).unwrap();
        assert_eq!(rings.len(), 2);
        assert_eq!(rings[0].len(), 3);
        assert_eq!(rings[0][0], [100.0, 200.0, -50.0]);
        assert_eq!(rings[1][2], [310.0, 410.0, -62.0]);
    }
}
