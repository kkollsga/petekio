//! `WellsView` — a lightweight, broadcastable, filterable borrow over a
//! project's wells.
//!
//! A [`WellsView`] holds borrows (`&Well`) into a [`GeoData`](super::GeoData)'s
//! well collection — never clones. It is the substrate behind the broadcast
//! ergonomic: narrow the set with [`filter`](WellsView::filter) /
//! [`tops`](WellsView::tops), then [`iter`](WellsView::iter) and reduce each
//! well to a `Stats`. The Python `geo.wells.filter(...).tops("Brent").ntg` chain
//! (Phase 7, via `__getattr__`) builds on exactly this.

use crate::core::Well;

/// A borrowed, filterable view over a set of wells (insertion order preserved).
/// Cheap to derive — each `filter`/`tops` produces a new view sharing the same
/// `&Well` borrows, with no well cloned.
pub struct WellsView<'a> {
    wells: Vec<&'a Well>,
}

impl<'a> WellsView<'a> {
    /// Build a view from a list of well borrows. Crate-internal: the public
    /// entry point is [`GeoData::wells`](super::GeoData::wells).
    pub(crate) fn new(wells: Vec<&'a Well>) -> WellsView<'a> {
        WellsView { wells }
    }

    /// A new view keeping only the wells for which `pred` holds, in order.
    pub fn filter(&self, pred: impl Fn(&Well) -> bool) -> WellsView<'a> {
        WellsView::new(self.wells.iter().copied().filter(|w| pred(w)).collect())
    }

    /// Iterate the wells of this view in insertion order.
    pub fn iter(&self) -> impl Iterator<Item = &Well> {
        self.wells.iter().copied()
    }

    /// A new view narrowed to the wells that *have* the named top (the main
    /// bore resolves it; case-insensitive, matching `Well::top`).
    pub fn tops(&self, name: &str) -> WellsView<'a> {
        self.filter(|w| w.top(name).is_some())
    }

    /// Number of wells in this view.
    pub fn len(&self) -> usize {
        self.wells.len()
    }

    /// Whether this view holds no wells.
    pub fn is_empty(&self) -> bool {
        self.wells.is_empty()
    }
}
