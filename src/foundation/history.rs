//! Shared operation-history support for domain objects.
//!
//! History is deliberately human-readable at this layer. It is not a full
//! provenance graph; it is the object's local audit trail for user-visible
//! loads, calculations, resampling, assignment, and masking operations.

/// Human-readable operation history shared by all value-bearing domain objects.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct OperationHistory {
    entries: Vec<String>,
}

impl OperationHistory {
    /// Empty history.
    pub fn new() -> Self {
        Self::default()
    }

    /// History initialized with one entry.
    pub fn from_entry(entry: impl Into<String>) -> Self {
        Self {
            entries: vec![entry.into()],
        }
    }

    /// Borrow the entries.
    pub fn entries(&self) -> &[String] {
        &self.entries
    }

    /// Clone entries as an owned vector for FFI/reporting.
    pub fn to_vec(&self) -> Vec<String> {
        self.entries.clone()
    }

    /// Append one operation entry.
    pub fn push(&mut self, entry: impl Into<String>) {
        self.entries.push(entry.into());
    }

    /// Append entries from another history with a label prefix, e.g. `rhs.*`.
    pub fn extend_prefixed(&mut self, prefix: &str, other: &OperationHistory) {
        self.entries.extend(
            other
                .entries
                .iter()
                .map(|entry| format!("{prefix}.{entry}")),
        );
    }

    /// Clone this history and append one entry.
    pub fn with_entry(&self, entry: impl Into<String>) -> Self {
        let mut out = self.clone();
        out.push(entry);
        out
    }
}

impl From<Vec<String>> for OperationHistory {
    fn from(entries: Vec<String>) -> Self {
        Self { entries }
    }
}

impl From<OperationHistory> for Vec<String> {
    fn from(history: OperationHistory) -> Self {
        history.entries
    }
}

/// Common history surface implemented by value-bearing domain objects.
pub trait HasHistory {
    /// Borrow the standardized history container.
    fn operation_history(&self) -> &OperationHistory;

    /// Mutably borrow the standardized history container.
    fn operation_history_mut(&mut self) -> &mut OperationHistory;

    /// Human-readable entries, suitable for direct user display.
    fn history(&self) -> &[String] {
        self.operation_history().entries()
    }

    /// Append one history entry.
    fn record_history(&mut self, entry: impl Into<String>) {
        self.operation_history_mut().push(entry);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_preserves_order_and_prefixes_sources() {
        let mut history = OperationHistory::from_entry("surface.constant(value=1)");
        history.push("surface.add_scalar(2)");

        let rhs = OperationHistory::from_entry("surface.constant(value=3)");
        history.extend_prefixed("rhs", &rhs);
        history.push("surface.plus(surface)");

        assert_eq!(
            history.entries(),
            &[
                "surface.constant(value=1)".to_string(),
                "surface.add_scalar(2)".to_string(),
                "rhs.surface.constant(value=3)".to_string(),
                "surface.plus(surface)".to_string(),
            ]
        );
    }
}
