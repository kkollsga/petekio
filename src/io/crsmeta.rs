//! Minimal `crsmeta.xml` sidecar reader. It extracts a CRS label only; petekIO
//! records the label for provenance and never reprojects coordinates.

use crate::foundation::{GeoError, Result};
use std::path::Path;

/// Read a CRS label from a `crsmeta.xml` sidecar.
pub fn load_label(path: &Path) -> Result<String> {
    let text = crate::io::decode_latin1(&std::fs::read(path)?);
    parse_label(&text)
        .ok_or_else(|| GeoError::Parse(format!("crsmeta '{}': no CRS label found", path.display())))
}

fn parse_label(text: &str) -> Option<String> {
    tag_value(text, "label")
        .or_else(|| tag_value(text, "crs"))
        .or_else(|| attr_value(text, "label"))
        .or_else(|| attr_value(text, "name"))
        .map(|s| xml_unescape(s.trim()).to_string())
        .filter(|s| !s.is_empty())
}

fn tag_value<'a>(text: &'a str, tag: &str) -> Option<&'a str> {
    let lower = text.to_ascii_lowercase();
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = lower.find(&open)? + open.len();
    let end = lower[start..].find(&close)? + start;
    Some(&text[start..end])
}

fn attr_value<'a>(text: &'a str, attr: &str) -> Option<&'a str> {
    let lower = text.to_ascii_lowercase();
    for quote in ['"', '\''] {
        let pat = format!("{attr}={quote}");
        let start = lower.find(&pat)? + pat.len();
        let end = lower[start..].find(quote)? + start;
        if end > start {
            return Some(&text[start..end]);
        }
    }
    None
}

fn xml_unescape(s: &str) -> std::borrow::Cow<'_, str> {
    if !s.contains('&') {
        return std::borrow::Cow::Borrowed(s);
    }
    std::borrow::Cow::Owned(
        s.replace("&amp;", "&")
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&quot;", "\"")
            .replace("&apos;", "'"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn parses_label_tag() {
        let p = std::env::temp_dir().join("petekio_crsmeta.xml");
        std::fs::File::create(&p)
            .unwrap()
            .write_all(b"<crsmeta><label>ED50 / UTM zone 31N</label></crsmeta>")
            .unwrap();
        assert_eq!(load_label(&p).unwrap(), "ED50 / UTM zone 31N");
    }
}
