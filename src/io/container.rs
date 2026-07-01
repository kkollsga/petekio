//! `io::container` — a **domain-agnostic** single-file section container.
//!
//! One file = `magic(4) + data_version(u32 LE) + header_len(u32 LE) + header
//! (JSON) + section blobs`. Each section is a `kind` + `name` + `tags` +
//! `version` + an opaque `payload`, stored `zstd`-compressed. The container
//! knows **nothing** about surfaces / wells / GeoData — it round-trips tagged,
//! versioned, kinded byte blobs and supports partial reads + `split` / `merge` /
//! `export`-by-tag that copy compressed blobs **byte-for-byte** (never
//! re-encoding a payload). Factored to lift into a shared toolkit unchanged;
//! only [`GeoError`] couples it to petekIO (swap on lift).
//!
//! Two-tier versioning: the 4th magic byte is the **hard** format version (a
//! mismatch is refused); `data_version` tracks the *caller's* DTO schema; the
//! JSON header evolves via serde defaults (soft). Per-section `version` lets an
//! opaque sidecar (e.g. petekSim's model) evolve independently.

use crate::foundation::{GeoError, Result};
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

/// Magic + hard format-version byte. A mismatch on the first 3 bytes or a newer
/// version byte is refused with a clear error.
const MAGIC: [u8; 4] = *b"PIO\x01";
const ZSTD_LEVEL: i32 = 3;

/// A section as the caller sees it: metadata + an **uncompressed** payload.
#[derive(Debug, Clone)]
pub struct Section {
    pub kind: String,
    pub name: String,
    pub tags: Vec<String>,
    pub version: u32,
    pub payload: Vec<u8>,
}

/// One entry in the JSON header index (metadata only — no payload).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    pub kind: String,
    pub name: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub version: u32,
    /// Offset of the compressed blob within the blob region (after the header).
    pub offset: u64,
    /// Compressed blob length.
    pub size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Header {
    format_version: u32,
    /// Opaque, caller-owned project metadata (unit, owner, timestamps, tags,
    /// strat_hints, …) — the container does not interpret it.
    #[serde(default)]
    app: serde_json::Value,
    sections: Vec<Entry>,
}

/// Write a container from uncompressed sections (compresses each payload).
pub fn write(
    path: &Path,
    app: &serde_json::Value,
    data_version: u32,
    sections: &[Section],
) -> Result<()> {
    let blobs: Result<Vec<(Entry, Vec<u8>)>> = sections
        .iter()
        .map(|s| {
            let blob = zstd::encode_all(s.payload.as_slice(), ZSTD_LEVEL)?;
            Ok((entry_of(s, 0, blob.len() as u64), blob))
        })
        .collect();
    write_frame(path, app, data_version, blobs?)
}

/// Write from **pre-compressed** blobs verbatim — the byte-lossless path used by
/// `split` / `merge` / `export`. `items` = `(entry, compressed blob)`; offsets
/// are recomputed, blobs copied untouched.
fn write_raw(
    path: &Path,
    app: &serde_json::Value,
    data_version: u32,
    items: Vec<(Entry, Vec<u8>)>,
) -> Result<()> {
    write_frame(path, app, data_version, items)
}

/// Common framing: compute offsets, serialize the header, write atomically
/// (temp file + rename) so a crash never corrupts an existing project.
fn write_frame(
    path: &Path,
    app: &serde_json::Value,
    data_version: u32,
    mut items: Vec<(Entry, Vec<u8>)>,
) -> Result<()> {
    let mut offset = 0u64;
    for (entry, blob) in &mut items {
        entry.offset = offset;
        entry.size = blob.len() as u64;
        offset += blob.len() as u64;
    }
    let header = Header {
        format_version: MAGIC[3] as u32,
        app: app.clone(),
        sections: items.iter().map(|(e, _)| e.clone()).collect(),
    };
    let header_bytes = serde_json::to_vec(&header).map_err(|e| GeoError::Parse(e.to_string()))?;

    let tmp = with_ext(path, "tmp");
    {
        let mut f = File::create(&tmp)?;
        f.write_all(&MAGIC)?;
        f.write_all(&data_version.to_le_bytes())?;
        f.write_all(&(header_bytes.len() as u32).to_le_bytes())?;
        f.write_all(&header_bytes)?;
        for (_, blob) in &items {
            f.write_all(blob)?;
        }
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// An opened container: header parsed, blobs read lazily by offset.
pub struct Reader {
    file: File,
    header: Header,
    data_version: u32,
    blob_base: u64,
}

/// Open a container, reading only the header (blobs are pulled on demand).
pub fn open(path: &Path) -> Result<Reader> {
    let mut file = File::open(path)?;
    let mut magic = [0u8; 4];
    file.read_exact(&mut magic)
        .map_err(|_| GeoError::Parse("not a .pproj file (truncated header)".into()))?;
    if magic[..3] != MAGIC[..3] {
        return Err(GeoError::Parse("not a .pproj file (bad magic)".into()));
    }
    if magic[3] > MAGIC[3] {
        return Err(GeoError::Parse(format!(
            "unsupported .pproj format version {} (this build reads ≤ {}) — re-save with a newer petekIO",
            magic[3], MAGIC[3]
        )));
    }
    let mut u32buf = [0u8; 4];
    file.read_exact(&mut u32buf)?;
    let data_version = u32::from_le_bytes(u32buf);
    file.read_exact(&mut u32buf)?;
    let header_len = u32::from_le_bytes(u32buf) as usize;
    let mut header_bytes = vec![0u8; header_len];
    file.read_exact(&mut header_bytes)
        .map_err(|_| GeoError::Parse(".pproj header truncated".into()))?;
    let header: Header =
        serde_json::from_slice(&header_bytes).map_err(|e| GeoError::Parse(e.to_string()))?;
    let blob_base = 12 + header_len as u64;
    Ok(Reader {
        file,
        header,
        data_version,
        blob_base,
    })
}

impl Reader {
    pub fn data_version(&self) -> u32 {
        self.data_version
    }
    pub fn app(&self) -> &serde_json::Value {
        &self.header.app
    }
    /// The header index — list contents **without** reading any blob.
    pub fn entries(&self) -> &[Entry] {
        &self.header.sections
    }
    fn entry(&self, name: &str) -> Option<&Entry> {
        self.header.sections.iter().find(|e| e.name == name)
    }

    /// The raw **compressed** blob for `name` (used by split/merge — verbatim).
    fn raw(&mut self, name: &str) -> Result<Vec<u8>> {
        let e = self
            .entry(name)
            .ok_or_else(|| GeoError::NotFound(format!("section '{name}'")))?
            .clone();
        let mut buf = vec![0u8; e.size as usize];
        self.file.seek(SeekFrom::Start(self.blob_base + e.offset))?;
        self.file
            .read_exact(&mut buf)
            .map_err(|_| GeoError::Parse(format!("section '{name}' truncated")))?;
        Ok(buf)
    }

    /// Read + decompress one section by name (partial load).
    pub fn read(&mut self, name: &str) -> Result<Section> {
        let e = self
            .entry(name)
            .ok_or_else(|| GeoError::NotFound(format!("section '{name}'")))?
            .clone();
        let blob = self.raw(name)?;
        let payload = zstd::decode_all(blob.as_slice())
            .map_err(|_| GeoError::Parse(format!("section '{name}' corrupt")))?;
        Ok(Section {
            kind: e.kind,
            name: e.name,
            tags: e.tags,
            version: e.version,
            payload,
        })
    }
}

/// Write a new container holding only the sections whose name passes `keep`,
/// copying their compressed blobs **byte-for-byte** (no re-encode). `merge`s the
/// header `app` from the source. This is the engine behind split / export.
pub fn filter_to(src: &Path, dst: &Path, keep: impl Fn(&Entry) -> bool) -> Result<()> {
    let mut r = open(src)?;
    let entries: Vec<Entry> = r.entries().iter().filter(|e| keep(e)).cloned().collect();
    let mut items = Vec::with_capacity(entries.len());
    for e in entries {
        let blob = r.raw(&e.name)?;
        items.push((e, blob));
    }
    let app = r.app().clone();
    let dv = r.data_version();
    write_raw(dst, &app, dv, items)
}

/// Merge `b`'s sections into `a` → `dst`, copying blobs verbatim. On a
/// `kind`+`name` clash, `b` wins (last-writer). `a`'s header `app` is kept.
pub fn merge_to(a: &Path, b: &Path, dst: &Path) -> Result<()> {
    let mut ra = open(a)?;
    let mut rb = open(b)?;
    let mut items: Vec<(Entry, Vec<u8>)> = Vec::new();
    let mut take = |r: &mut Reader| -> Result<()> {
        for e in r.entries().to_vec() {
            items.retain(|(x, _)| !(x.kind == e.kind && x.name == e.name));
            let blob = r.raw(&e.name)?;
            items.push((e, blob));
        }
        Ok(())
    };
    take(&mut ra)?;
    take(&mut rb)?;
    let app = ra.app().clone();
    let dv = ra.data_version();
    write_raw(dst, &app, dv, items)
}

fn entry_of(s: &Section, offset: u64, size: u64) -> Entry {
    Entry {
        kind: s.kind.clone(),
        name: s.name.clone(),
        tags: s.tags.clone(),
        version: s.version,
        offset,
        size,
    }
}

fn with_ext(path: &Path, ext: &str) -> std::path::PathBuf {
    let mut p = path.to_path_buf();
    let cur = p.extension().and_then(|e| e.to_str()).unwrap_or("");
    p.set_extension(format!("{cur}.{ext}"));
    p
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sec(kind: &str, name: &str, tags: &[&str], version: u32, payload: &[u8]) -> Section {
        Section {
            kind: kind.into(),
            name: name.into(),
            tags: tags.iter().map(|s| s.to_string()).collect(),
            version,
            payload: payload.to_vec(),
        }
    }

    fn tmp(tag: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("pio_{tag}_{}.pproj", std::process::id()))
    }

    #[test]
    fn round_trips_sections_and_app_metadata() {
        let p = tmp("rt");
        let app = json!({"unit": "m", "owner": "kk", "tags": ["cerisa"]});
        // include NaN bytes in a payload — the container is byte-opaque.
        let nan = f64::NAN.to_le_bytes();
        let secs = [
            sec("surface", "top", &["cerisa"], 0, &nan),
            sec(
                "model",
                "model/seg/props",
                &["cerisa"],
                7,
                b"\x00\xffopaque",
            ),
        ];
        write(&p, &app, 2, &secs).unwrap();

        let mut r = open(&p).unwrap();
        assert_eq!(r.data_version(), 2);
        assert_eq!(r.app()["owner"], "kk");
        assert_eq!(r.entries().len(), 2);
        let m = r.read("model/seg/props").unwrap();
        assert_eq!(m.payload, b"\x00\xffopaque"); // opaque bytes preserved
        assert_eq!(m.version, 7);
        assert_eq!(r.read("top").unwrap().payload, nan); // NaN bytes intact
        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn rejects_bad_magic_and_newer_version() {
        let p = tmp("bad");
        std::fs::write(&p, b"XXXX........").unwrap();
        assert!(open(&p).is_err());
        std::fs::write(&p, b"PIO\xfe....").unwrap(); // version 254 > ours
        assert!(open(&p).is_err());
        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn export_by_tag_is_byte_lossless_subset() {
        let (src, dst) = (tmp("exp_src"), tmp("exp_dst"));
        write(
            &src,
            &json!({}),
            1,
            &[
                sec("surface", "a", &["keep"], 0, b"AAA"),
                sec("surface", "b", &["drop"], 0, b"BBB"),
                sec("model", "model/s/props", &["keep"], 3, b"\x01\x02\x03"),
            ],
        )
        .unwrap();
        filter_to(&src, &dst, |e| e.tags.iter().any(|t| t == "keep")).unwrap();

        let mut r = open(&dst).unwrap();
        let names: Vec<_> = r.entries().iter().map(|e| e.name.clone()).collect();
        assert_eq!(names, ["a", "model/s/props"]); // only tagged, order preserved
                                                   // model section survives byte-for-byte with its own version.
        let m = r.read("model/s/props").unwrap();
        assert_eq!((m.version, m.payload), (3, vec![1, 2, 3]));
    }

    #[test]
    fn merge_unions_and_last_wins_on_clash() {
        let (a, b, dst) = (tmp("m_a"), tmp("m_b"), tmp("m_d"));
        write(
            &a,
            &json!({"owner": "a"}),
            1,
            &[
                sec("surface", "x", &[], 0, b"OLD"),
                sec("surface", "only_a", &[], 0, b"A"),
            ],
        )
        .unwrap();
        write(
            &b,
            &json!({"owner": "b"}),
            1,
            &[
                sec("surface", "x", &[], 0, b"NEW"),
                sec("surface", "only_b", &[], 0, b"B"),
            ],
        )
        .unwrap();
        merge_to(&a, &b, &dst).unwrap();

        let mut r = open(&dst).unwrap();
        let names: Vec<_> = r.entries().iter().map(|e| e.name.clone()).collect();
        assert_eq!(names, ["only_a", "x", "only_b"]); // b's x replaced a's in place
        assert_eq!(r.read("x").unwrap().payload, b"NEW"); // last-writer wins
        assert_eq!(r.app()["owner"], "a"); // a's app kept
    }
}
