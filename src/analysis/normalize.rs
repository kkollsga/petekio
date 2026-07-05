//! `normalize` — canonicalize heterogeneous inputs into model-ready form:
//! LAS mnemonic aliasing (`PHI`/`PHIE`/`EFFPHI` → canonical), formation & well
//! name-maps, and unit harmonisation (length units → the project unit; percent →
//! fraction). The first half of the path from loaded data to
//! [`ModelInputs`](super::model_inputs::ModelInputs).
//!
//! Open/closed: the mnemonic table and unit tables are extended by adding rows,
//! never by editing call sites; unknown inputs pass through (vintage-tag stripped,
//! original case kept) so an unrecognised curve is preserved, not dropped.

use crate::foundation::{GeoError, Result, Unit};
use std::collections::HashMap;

/// Canonical curve mnemonic for a raw LAS mnemonic (case-insensitive, trimmed).
///
/// Maps common vendor variants of the *same* physical curve to one canonical
/// name. Physically distinct curves (neutron `NPHI` vs effective `PHIE`,
/// effective `SW` vs total `SWT`) keep distinct canonicals. An unrecognised
/// mnemonic is returned vintage-stripped (original case) — never dropped.
pub fn canonical_mnemonic(raw: &str) -> String {
    // Strip a trailing vintage tag (`_2025`, `_2024`, …) before matching, so
    // Petrel comp-log names like `PHIE_2025`/`SW_2025` resolve. Semantic variants
    // the table can't guess (`NTG_PhieLam` vs `NTG_VShale`, `PERM_Lam`) are left to
    // a user alias map — see [`canonical_mnemonic_with`].
    let stem = strip_vintage(raw.trim());
    let key = stem.to_ascii_uppercase();
    let canonical = match key.as_str() {
        // Effective porosity.
        "PHIE" | "PHI" | "PHI_E" | "EFFPHI" | "PHIEF" => "PHIE",
        // Total porosity (kept distinct from effective).
        "PHIT" | "PHI_T" | "TOTPHI" => "PHIT",
        // Neutron porosity.
        "NPHI" | "TNPH" | "NEU" | "CNL" => "NPHI",
        // Effective water saturation.
        "SW" | "SWE" | "SUWI" | "SW_E" => "SW",
        // Total water saturation (kept distinct from effective SW).
        "SWT" | "SW_T" | "SWTOT" => "SWT",
        // Gamma ray.
        "GR" | "GRC" | "SGR" | "CGR" | "GAMMA" => "GR",
        // Shale/clay volume.
        "VSH" | "VCL" | "VSHALE" | "VCLAY" | "VSHGR" => "VSH",
        // Permeability.
        "PERM" | "K" | "KLOGH" | "KH" | "PERMH" => "PERM",
        // Bulk density.
        "RHOB" | "DEN" | "DENS" | "ZDEN" => "RHOB",
        // Sonic / interval transit time.
        "DT" | "AC" | "SONIC" | "DTCO" => "DT",
        // Deep resistivity.
        "RT" | "RES" | "ILD" | "LLD" | "RDEP" => "RT",
        // Net-to-gross.
        "NTG" | "NET_GROSS" | "N_G" => "NTG",
        _ => return stem.to_string(),
    };
    canonical.to_string()
}

/// Canonical mnemonic honouring a **user alias map** first (exact, case-
/// insensitive), then falling back to [`canonical_mnemonic`]. The alias map is
/// how a project resolves the choices the built-in table can't guess — e.g.
/// which net-to-gross is canonical (`NTG_PhieLam` vs `NTG_VShale` → `NTG`),
/// `PERM_Lam_2025` → `PERM`, or any vendor-specific name.
pub fn canonical_mnemonic_with(raw: &str, aliases: &NameMap) -> String {
    aliases.get(raw).unwrap_or_else(|| canonical_mnemonic(raw))
}

/// Strip a trailing `_<4-digit year>` vintage tag (e.g. `PHIE_2025` → `PHIE`),
/// leaving everything else untouched.
fn strip_vintage(s: &str) -> &str {
    if let Some((head, tail)) = s.rsplit_once('_') {
        if tail.len() == 4 && tail.bytes().all(|b| b.is_ascii_digit()) {
            return head;
        }
    }
    s
}

/// Parse a length-unit string (case-insensitive) to a [`Unit`], or `None` if
/// unrecognised.
pub fn parse_length_unit(s: &str) -> Option<Unit> {
    match s.trim().to_ascii_lowercase().as_str() {
        "m" | "metre" | "metres" | "meter" | "meters" => Some(Unit::Metres),
        "ft" | "f" | "feet" | "foot" => Some(Unit::Feet),
        _ => None,
    }
}

/// True if a unit string denotes percent — a fractional curve in these units
/// must be divided by 100 to reach a `[0,1]` fraction.
pub fn is_percent_unit(s: &str) -> bool {
    matches!(
        s.trim().to_ascii_lowercase().as_str(),
        "%" | "percent" | "pct" | "p.u." | "pu"
    )
}

/// Harmonise a fractional value given its unit string: percent → fraction,
/// otherwise unchanged. `NaN` propagates.
pub fn harmonise_fraction(value: f64, unit: &str) -> f64 {
    if is_percent_unit(unit) {
        value / 100.0
    } else {
        value
    }
}

/// Convert a depth/length `value` from `from` units to `to` (the project unit).
/// Thin wrapper over [`Unit::convert`] for symmetry with the other passes.
pub fn harmonise_length(value: f64, from: Unit, to: Unit) -> f64 {
    from.convert(value, to)
}

/// A case-insensitive alias → canonical name map for formations or wells.
/// Lookup is identity (the input, trimmed) when no alias is registered, so an
/// unmapped name is preserved rather than lost.
///
/// A value type: it serialises (a scenario/ingest spec is a savable file),
/// compares by value, and [`Display`](std::fmt::Display)s as its alias rows —
/// the mirror behind the Python `IngestSpec`'s alias map.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct NameMap {
    map: HashMap<String, String>,
}

impl std::fmt::Display for NameMap {
    /// `raw -> canonical` rows in sorted key order (empty map → `(no aliases)`).
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.map.is_empty() {
            return write!(f, "(no aliases)");
        }
        let mut rows: Vec<(&String, &String)> = self.map.iter().collect();
        rows.sort_by(|a, b| a.0.cmp(b.0));
        let body = rows
            .iter()
            .map(|(k, v)| format!("{k} -> {v}"))
            .collect::<Vec<_>>()
            .join(", ");
        write!(f, "{body}")
    }
}

/// Soft lithostratigraphic ordering hints as `(above, below)` name tokens
/// (possibly partial). A declarative value applied at well-tops load: each token
/// is resolved to an actual top name **at apply time** (loud error naming the
/// bad token), then honoured only where the data leaves the pair unordered.
///
/// The value behind the Python `IngestSpec`'s `strat_hints`: it serialises,
/// compares by value, and [`Display`](std::fmt::Display)s as its `A < B` rows.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct StratHints {
    pairs: Vec<(String, String)>,
}

impl StratHints {
    /// An empty hint set.
    pub fn new() -> Self {
        Self::default()
    }

    /// Build from `(above, below)` pairs (`above` shallower than `below`).
    pub fn from_pairs(pairs: impl IntoIterator<Item = (String, String)>) -> Self {
        Self {
            pairs: pairs.into_iter().collect(),
        }
    }

    /// Push one `above` shallower-than `below` hint.
    pub fn push(&mut self, above: impl Into<String>, below: impl Into<String>) {
        self.pairs.push((above.into(), below.into()));
    }

    /// Parse+append a shorthand hint: `"A < B"` (A above B) / `"A > B"` (A below
    /// B). `Err` if the spec carries neither `<` nor `>` or has an empty side.
    pub fn push_spec(&mut self, spec: &str) -> Result<()> {
        let (above, below) = if let Some((l, r)) = spec.split_once('<') {
            (l.trim(), r.trim())
        } else if let Some((l, r)) = spec.split_once('>') {
            (r.trim(), l.trim())
        } else {
            return Err(GeoError::Parse(format!(
                "strat hint '{spec}' must contain '<' (above) or '>' (below)"
            )));
        };
        if above.is_empty() || below.is_empty() {
            return Err(GeoError::Parse(format!(
                "strat hint '{spec}' has an empty side"
            )));
        }
        self.push(above, below);
        Ok(())
    }

    /// The `(above, below)` token pairs, in insertion order.
    pub fn pairs(&self) -> &[(String, String)] {
        &self.pairs
    }

    /// Whether any hint is registered.
    pub fn is_empty(&self) -> bool {
        self.pairs.is_empty()
    }
}

impl std::fmt::Display for StratHints {
    /// `A < B` rows joined by `, ` (empty → `(no hints)`).
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.pairs.is_empty() {
            return write!(f, "(no hints)");
        }
        let body = self
            .pairs
            .iter()
            .map(|(a, b)| format!("{a} < {b}"))
            .collect::<Vec<_>>()
            .join(", ");
        write!(f, "{body}")
    }
}

impl NameMap {
    /// An empty map (every name canonicalises to itself).
    pub fn new() -> Self {
        Self::default()
    }

    /// Build from `(alias, canonical)` pairs.
    pub fn from_pairs(pairs: impl IntoIterator<Item = (String, String)>) -> Self {
        let mut m = Self::new();
        for (alias, canonical) in pairs {
            m.insert(alias, canonical);
        }
        m
    }

    /// Register an `alias` (matched case-insensitively) → `canonical` name.
    pub fn insert(&mut self, alias: impl Into<String>, canonical: impl Into<String>) {
        self.map
            .insert(alias.into().trim().to_ascii_lowercase(), canonical.into());
    }

    /// The canonical name for `name`, or `name` trimmed if no alias is registered.
    pub fn canonical(&self, name: &str) -> String {
        self.get(name).unwrap_or_else(|| name.trim().to_string())
    }

    /// The registered canonical for `name` (case-insensitive), or `None` if
    /// unmapped — unlike [`canonical`](Self::canonical), no identity fallback, so
    /// callers can fall through to other resolution (e.g. the mnemonic table).
    pub fn get(&self, name: &str) -> Option<String> {
        self.map
            .get(name.trim().to_ascii_lowercase().as_str())
            .cloned()
    }

    /// The registered `(alias, canonical)` pairs in sorted-alias order (aliases
    /// are stored lowercased). For serialising the map as a value / round-trip.
    pub fn pairs(&self) -> Vec<(String, String)> {
        let mut rows: Vec<(String, String)> = self
            .map
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        rows.sort_by(|a, b| a.0.cmp(&b.0));
        rows
    }

    /// Whether any alias is registered.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    #[test]
    fn namemap_value_affordances() {
        let nm = NameMap::from_pairs([
            ("PHIE_2025".to_string(), "PHIE".to_string()),
            ("SW_2025".to_string(), "SW".to_string()),
        ]);
        // pairs() is sorted, lowercased aliases.
        assert_eq!(
            nm.pairs(),
            vec![
                ("phie_2025".to_string(), "PHIE".to_string()),
                ("sw_2025".to_string(), "SW".to_string()),
            ]
        );
        assert_eq!(format!("{nm}"), "phie_2025 -> PHIE, sw_2025 -> SW");
        assert_eq!(format!("{}", NameMap::new()), "(no aliases)");
        // serde + value equality.
        let json = serde_json::to_string(&nm).unwrap();
        assert_eq!(serde_json::from_str::<NameMap>(&json).unwrap(), nm);
    }

    #[test]
    fn strat_hints_value_affordances() {
        let mut h = StratHints::from_pairs([("A".to_string(), "B".to_string())]);
        h.push_spec("C < D").unwrap();
        h.push_spec("F > E").unwrap(); // F below E => (E, F)
        assert_eq!(
            h.pairs(),
            vec![
                ("A".to_string(), "B".to_string()),
                ("C".to_string(), "D".to_string()),
                ("E".to_string(), "F".to_string()),
            ]
        );
        assert_eq!(format!("{h}"), "A < B, C < D, E < F");
        assert!(StratHints::new().is_empty());
        assert!(h.push_spec("no operator").is_err());
        let json = serde_json::to_string(&h).unwrap();
        assert_eq!(serde_json::from_str::<StratHints>(&json).unwrap(), h);
    }

    #[test]
    fn mnemonic_aliases_resolve_case_insensitively() {
        assert_eq!(canonical_mnemonic("phi"), "PHIE");
        assert_eq!(canonical_mnemonic(" Suwi "), "SW");
        assert_eq!(canonical_mnemonic("VCL"), "VSH");
        // physically distinct porosities stay distinct
        assert_eq!(canonical_mnemonic("NPHI"), "NPHI");
        assert_eq!(canonical_mnemonic("PHIT"), "PHIT");
        // unknown passes through (vintage-stripped) — not forced uppercase
        assert_eq!(canonical_mnemonic("NTG_PhieLam"), "NTG_PhieLam");
    }

    #[test]
    fn vintage_suffix_is_stripped() {
        assert_eq!(canonical_mnemonic("PHIE_2025"), "PHIE");
        assert_eq!(canonical_mnemonic("SW_2025"), "SW");
        assert_eq!(canonical_mnemonic("VShale_2025"), "VSH");
        // effective vs total water saturation stay distinct (was a wrong SWT→SW)
        assert_eq!(canonical_mnemonic("SWT_2025"), "SWT");
        assert_eq!(canonical_mnemonic("PHIT_2025"), "PHIT");
        // a non-year trailing tag is NOT stripped
        assert_eq!(canonical_mnemonic("PERM_Lam"), "PERM_Lam");
    }

    #[test]
    fn user_alias_map_resolves_the_unguessable() {
        let aliases = NameMap::from_pairs([
            ("NTG_PhieLam".to_string(), "NTG".to_string()),
            ("PERM_Lam_2025".to_string(), "PERM".to_string()),
        ]);
        // user map wins (the NTG-collision choice)
        assert_eq!(canonical_mnemonic_with("NTG_PhieLam", &aliases), "NTG");
        assert_eq!(canonical_mnemonic_with("perm_lam_2025", &aliases), "PERM"); // case-insensitive
                                                                                // unmapped falls through to the table (+ vintage strip)
        assert_eq!(canonical_mnemonic_with("PHIE_2025", &aliases), "PHIE");
        assert_eq!(NameMap::new().get("nope"), None);
    }

    #[test]
    fn length_units_parse() {
        assert_eq!(parse_length_unit("M"), Some(Unit::Metres));
        assert_eq!(parse_length_unit("feet"), Some(Unit::Feet));
        assert_eq!(parse_length_unit("furlong"), None);
    }

    #[test]
    fn percent_harmonises_to_fraction() {
        assert!(is_percent_unit("%"));
        assert!(!is_percent_unit("v/v"));
        assert_relative_eq!(harmonise_fraction(25.0, "%"), 0.25);
        assert_relative_eq!(harmonise_fraction(0.25, "v/v"), 0.25);
        assert!(harmonise_fraction(f64::NAN, "%").is_nan());
    }

    #[test]
    fn length_harmonises_via_unit() {
        assert_relative_eq!(harmonise_length(100.0, Unit::Feet, Unit::Metres), 30.48);
    }

    #[test]
    fn name_map_is_identity_for_unknowns() {
        let m = NameMap::from_pairs([
            ("Brent Gp".to_string(), "Brent".to_string()),
            ("DUNLIN GROUP".to_string(), "Dunlin".to_string()),
        ]);
        assert_eq!(m.canonical("brent gp"), "Brent"); // case-insensitive alias
        assert_eq!(m.canonical("Dunlin Group"), "Dunlin");
        assert_eq!(m.canonical(" Statfjord "), "Statfjord"); // unmapped → trimmed identity
    }
}
