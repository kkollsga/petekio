//! Durable metadata for named surface property lanes.

use crate::foundation::{GeoError, Result};
use indexmap::IndexMap;

/// How a surface attribute is rendered and interpreted by generic consumers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AttributeKind {
    Continuous,
    Categorical,
}

/// One optional categorical-code label and colour.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CodeRecord {
    pub label: Option<String>,
    pub color: Option<String>,
}

impl CodeRecord {
    pub fn new(label: Option<String>, color: Option<String>) -> Result<Self> {
        let mut out = Self { label, color };
        if let Some(color) = &mut out.color {
            color.make_ascii_uppercase();
        }
        out.validate()?;
        Ok(out)
    }

    fn validate(&self) -> Result<()> {
        if self
            .label
            .as_deref()
            .is_some_and(|label| label.is_empty() || label != label.trim())
        {
            return Err(GeoError::Parse(
                "attribute code label must be null or a non-empty, trimmed string".into(),
            ));
        }
        if let Some(color) = &self.color {
            let bytes = color.as_bytes();
            if bytes.len() != 7 || bytes[0] != b'#' || !bytes[1..].iter().all(u8::is_ascii_hexdigit)
            {
                return Err(GeoError::Parse(format!(
                    "attribute code color '{color}' must be #RRGGBB or null"
                )));
            }
        }
        Ok(())
    }
}

/// Canonical metadata record for one named surface attribute.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AttributeMetadata {
    pub id: String,
    pub label: String,
    pub kind: AttributeKind,
    pub units: Option<String>,
    pub codes: Option<IndexMap<String, CodeRecord>>,
}

impl AttributeMetadata {
    pub fn new(
        id: impl Into<String>,
        label: impl Into<String>,
        kind: AttributeKind,
        units: Option<String>,
        codes: Option<IndexMap<String, CodeRecord>>,
    ) -> Result<Self> {
        let out = Self {
            id: id.into(),
            label: label.into(),
            kind,
            units,
            codes,
        };
        out.validate()?;
        Ok(out)
    }

    /// Metadata for a legacy values-only attribute authoring call.
    pub fn continuous(id: impl Into<String>) -> Result<Self> {
        let id = id.into();
        Self::new(id.clone(), id, AttributeKind::Continuous, None, None)
    }

    pub fn validate(&self) -> Result<()> {
        if self.id.is_empty()
            || self.id.as_str() != self.id.trim()
            || self.label.is_empty()
            || self.label.as_str() != self.label.trim()
        {
            return Err(GeoError::Parse(
                "attribute metadata id and label must be non-empty, trimmed strings".into(),
            ));
        }
        if self
            .units
            .as_deref()
            .is_some_and(|units| units.is_empty() || units != units.trim())
        {
            return Err(GeoError::Parse(
                "attribute metadata units must be null or a non-empty, trimmed string".into(),
            ));
        }
        if self.kind == AttributeKind::Continuous && self.codes.is_some() {
            return Err(GeoError::Parse(
                "continuous attribute metadata must use codes=null".into(),
            ));
        }
        if let Some(codes) = &self.codes {
            for (code, record) in codes {
                let parsed = code.parse::<i64>().map_err(|_| {
                    GeoError::Parse(format!(
                        "attribute code key '{code}' must be a canonical integer string"
                    ))
                })?;
                if parsed.to_string() != *code {
                    return Err(GeoError::Parse(format!(
                        "attribute code key '{code}' must be canonical integer string '{}'",
                        parsed
                    )));
                }
                record.validate()?;
            }
        }
        Ok(())
    }

    /// Canonicalize only presentation text from an already-persisted v2 record.
    /// IDs are semantic keys and are deliberately never rewritten.
    pub(crate) fn migrate_persisted_text(&mut self) {
        self.label = self.label.trim().to_string();
        if let Some(units) = &mut self.units {
            *units = units.trim().to_string();
        }
        if let Some(codes) = &mut self.codes {
            for record in codes.values_mut() {
                if let Some(label) = &mut record.label {
                    *label = label.trim().to_string();
                }
                if let Some(color) = &mut record.color {
                    color.make_ascii_uppercase();
                }
            }
        }
    }
}

/// Values plus their durable metadata. Kept crate-private so each surface level
/// exposes the same stable values-oriented public API.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct AttributeLane<T> {
    pub(crate) metadata: AttributeMetadata,
    pub(crate) values: T,
}

impl<T> AttributeLane<T> {
    pub(crate) fn new(metadata: AttributeMetadata, values: T) -> Result<Self> {
        metadata.validate()?;
        Ok(Self { metadata, values })
    }
}

pub(crate) fn check_metadata_name(name: &str, metadata: &AttributeMetadata) -> Result<()> {
    metadata.validate()?;
    if metadata.id != name {
        return Err(GeoError::Parse(format!(
            "attribute metadata id '{}' must match lane name '{name}'",
            metadata.id
        )));
    }
    Ok(())
}

pub(crate) fn validate_attribute_values<'a>(
    metadata: &AttributeMetadata,
    values: impl IntoIterator<Item = &'a f64>,
) -> Result<()> {
    if metadata.kind == AttributeKind::Categorical
        && values
            .into_iter()
            .any(|value| value.is_finite() && value.fract() != 0.0)
    {
        return Err(GeoError::Parse(format!(
            "categorical attribute '{}' values must be integers or NaN",
            metadata.id
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_canonical_categorical_codes() {
        let mut codes = IndexMap::new();
        codes.insert(
            "1".into(),
            CodeRecord::new(Some("Sand".into()), Some("#eda100".into())).unwrap(),
        );
        assert_eq!(codes["1"].color.as_deref(), Some("#EDA100"));
        assert!(AttributeMetadata::new(
            "facies",
            "Facies",
            AttributeKind::Categorical,
            None,
            Some(codes.clone()),
        )
        .is_ok());
        codes.insert("01".into(), CodeRecord::new(None, None).unwrap());
        assert!(AttributeMetadata::new(
            "facies",
            "Facies",
            AttributeKind::Categorical,
            None,
            Some(codes),
        )
        .is_err());
    }

    #[test]
    fn rejects_noncanonical_descriptor_strings() {
        for (id, label, units) in [
            (" ", "Label", None),
            ("id", "\t", None),
            ("id", "Label", Some(" \n".into())),
            (" id", "Label", None),
            ("id", "Label ", None),
            ("id", "Label", Some(" m ".into())),
        ] {
            assert!(
                AttributeMetadata::new(id, label, AttributeKind::Continuous, units, None,).is_err()
            );
        }
        assert!(CodeRecord::new(Some(" Sand ".into()), None).is_err());
        assert!(CodeRecord::new(Some(String::new()), None).is_err());
        assert!(CodeRecord::new(None, None).is_ok());
    }
}
