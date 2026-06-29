//! Shared helpers for integration tests that read **external** test data (kept
//! out of the repo, in the data folder). Resolve the root via `PETEKIO_TEST_DATA`
//! (else a default path) and let a test skip cleanly when the data isn't present
//! (e.g. in CI), so its absence is a no-op rather than a failure.

use std::path::PathBuf;

/// Root of the external petekIO test fixtures.
pub fn data_root() -> PathBuf {
    std::env::var("PETEKIO_TEST_DATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from("/Volumes/EksternalHome/Data/modellingProject/petekio-fixtures")
        })
}

/// `Some(path)` to `root/<rel>` when the data root exists, else `None` after
/// printing a skip notice. Use as: `let Some(dir) = require("…") else { return };`.
pub fn require(rel: &str) -> Option<PathBuf> {
    let root = data_root();
    if !root.exists() {
        eprintln!(
            "SKIP: external test data not found at {} (set PETEKIO_TEST_DATA)",
            root.display()
        );
        return None;
    }
    Some(root.join(rel))
}
