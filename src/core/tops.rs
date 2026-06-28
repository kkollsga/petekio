//! `Top` → `Interval` — formation tops and the depth interval each defines.
//!
//! A [`Top`] marks a formation entry at a measured depth. Tops are held
//! per-sidetrack, sorted by MD; the *interval* a top names runs from its MD down
//! to the **next** top's MD (or, for the deepest top, to total depth — the
//! active trajectory's `md_range().1`). An [`Interval`] reduces a log to a
//! [`Stats`](crate::foundation::Stats) over exactly that window — the engine
//! behind `well.top("Brent")?.log("NTG")?.stats()`.
//!
//! **API note:** `API.md` locks `Interval { name, top_md, base_md }`. To resolve
//! `Interval::log` (clip a log to the interval) the value must reach the
//! sidetrack's logs, so the struct additionally carries a private `&'a [Log]`
//! borrow and a lifetime `Interval<'a>`. This is the allowed lifetime refinement
//! flagged in the Phase-4 brief: the public fields are unchanged.

use crate::core::log::{Log, LogView};
use crate::foundation::Result;
use std::path::Path;

/// A formation top: a name and its entry measured depth.
#[derive(Debug, Clone, PartialEq)]
pub struct Top {
    /// Formation / marker name (e.g. `"Brent"`).
    pub name: String,
    /// Entry measured depth.
    pub md: f64,
}

impl Top {
    /// A top named `name` entering at measured depth `md`.
    pub fn new(name: impl Into<String>, md: f64) -> Top {
        Top {
            name: name.into(),
            md,
        }
    }

    /// Load tops from a headered CSV, taking the marker name from `name_col` and
    /// the measured depth from `md_col` (matched by header name).
    pub fn load_csv(path: impl AsRef<Path>, name_col: &str, md_col: &str) -> Result<Vec<Top>> {
        let recs = crate::io::tops::load(path.as_ref(), name_col, md_col)?;
        Ok(recs.into_iter().map(|r| Top::new(r.name, r.md)).collect())
    }
}

/// The depth interval a [`Top`] names: `[top_md, base_md)`, where `base_md` is
/// the next top's MD (or total depth for the deepest top). Borrows the owning
/// sidetrack's logs so [`log`](Self::log) can clip them to this window.
#[derive(Debug, Clone)]
pub struct Interval<'a> {
    /// The top's name.
    pub name: String,
    /// Interval top (entry) MD.
    pub top_md: f64,
    /// Interval base MD — the next top's MD, or total depth.
    pub base_md: f64,
    /// Private borrow of the sidetrack's logs, for `log()` resolution.
    logs: &'a [Log],
}

impl<'a> Interval<'a> {
    /// Build an interval borrowing `logs` for log resolution.
    pub(crate) fn new(name: String, top_md: f64, base_md: f64, logs: &'a [Log]) -> Interval<'a> {
        Interval {
            name,
            top_md,
            base_md,
            logs,
        }
    }

    /// The log `mnemonic` (case-insensitive) clipped to `[top_md, base_md)`, or
    /// `None` if no such log is present.
    pub fn log(&self, mnemonic: &str) -> Option<LogView<'a>> {
        let log = self
            .logs
            .iter()
            .find(|l| l.mnemonic.eq_ignore_ascii_case(mnemonic))?;
        Some(log.clip(self.top_md, self.base_md))
    }

    /// The interval thickness in measured depth (`base_md - top_md`).
    pub fn thickness_md(&self) -> f64 {
        self.base_md - self.top_md
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thickness_is_base_minus_top() {
        let i = Interval::new("Brent".into(), 2400.0, 2480.0, &[]);
        assert_eq!(i.thickness_md(), 80.0);
    }
}
