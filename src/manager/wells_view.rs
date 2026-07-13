//! `WellsView` — a lightweight, broadcastable, filterable borrow over a
//! project's wells.
//!
//! A [`WellsView`] holds borrows (`&Well`) into a [`GeoData`](super::GeoData)'s
//! well collection — never clones. It is the substrate behind the broadcast
//! ergonomic: narrow the set with [`filter`](WellsView::filter) /
//! [`tops`](WellsView::tops), then [`iter`](WellsView::iter) and reduce each
//! well to a `Stats`. The Python `geo.wells.filter(...).tops("Brent").ntg` chain
//! (Phase 7, via `__getattr__`) builds on exactly this.

use crate::core::{IntersectableSurface, SurfaceIntersection, Well};
use crate::foundation::Result;

/// One skipped or failed bore in a project-wide intersection report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntersectionDiagnostic {
    pub well: String,
    pub bore: String,
    pub reason: String,
    pub message: String,
}

/// Project-aware aggregate of well/surface intersections.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WellIntersectionSet {
    pub hits: Vec<SurfaceIntersection>,
    pub skipped: Vec<IntersectionDiagnostic>,
    pub failed: Vec<IntersectionDiagnostic>,
}

impl WellIntersectionSet {
    /// `(hits, skipped, failed)` counts.
    pub fn summary(&self) -> (usize, usize, usize) {
        (self.hits.len(), self.skipped.len(), self.failed.len())
    }
}

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

    /// Iterate the wells of this view in insertion order. Yielded borrows are
    /// tied to the project (`'a`), not to this view, so they outlive a
    /// temporary `geo.wells()`.
    pub fn iter(&self) -> impl Iterator<Item = &'a Well> {
        self.wells.clone().into_iter()
    }

    /// A new view narrowed to the wells that *have* the named top (the main
    /// bore resolves it; case-insensitive, matching `Well::top`).
    pub fn tops(&self, name: &str) -> WellsView<'a> {
        self.filter(|w| w.top(name).is_some())
    }

    /// Evaluate every trajectory-bearing bore and keep at most one hit per
    /// bore. No-hit bores are skipped; ambiguous/coplanar bores are reported as
    /// failures with guidance. Other wells continue evaluating.
    pub fn intersection<S: IntersectableSurface + ?Sized>(
        &self,
        surface: &S,
        tolerance: f64,
    ) -> Result<WellIntersectionSet> {
        self.evaluate(surface, tolerance, false)
    }

    /// Evaluate every trajectory-bearing bore and return all crossings.
    pub fn intersections<S: IntersectableSurface + ?Sized>(
        &self,
        surface: &S,
        tolerance: f64,
    ) -> Result<WellIntersectionSet> {
        self.evaluate(surface, tolerance, true)
    }

    fn evaluate<S: IntersectableSurface + ?Sized>(
        &self,
        surface: &S,
        tolerance: f64,
        all: bool,
    ) -> Result<WellIntersectionSet> {
        // Materialise/validate the surface once before entering the per-bore
        // diagnostic loop. A malformed surface is a request failure, not a bore
        // failure. The domain calls below currently rebuild the mesh; keeping
        // this explicit validation preserves the all-or-report contract.
        let _ = surface.intersection_mesh()?;
        let mut out = WellIntersectionSet::default();
        for well in &self.wells {
            let mut any_trajectory = false;
            for bore in well.sidetracks() {
                if bore.trajectories().is_empty() {
                    continue;
                }
                any_trajectory = true;
                let result = if all {
                    bore.intersections(surface, tolerance)
                } else {
                    bore.intersection(surface, tolerance)
                        .map(|hit| hit.into_iter().collect())
                };
                match result {
                    Ok(hits) if hits.is_empty() => out.skipped.push(IntersectionDiagnostic {
                        well: well.id.clone(),
                        bore: bore.label.clone(),
                        reason: "outside_or_no_intersection".into(),
                        message: "trajectory does not intersect the finite surface geometry".into(),
                    }),
                    Ok(hits) => out.hits.extend(
                        hits.into_iter()
                            .map(|hit| hit.identify(Some(&well.id), Some(&bore.label), None)),
                    ),
                    Err(error) => out.failed.push(IntersectionDiagnostic {
                        well: well.id.clone(),
                        bore: bore.label.clone(),
                        reason: if error.to_string().contains("coplanar") {
                            "coplanar".into()
                        } else if error.to_string().contains("crosses the surface") {
                            "multiple_intersections".into()
                        } else {
                            "intersection_error".into()
                        },
                        message: error.to_string(),
                    }),
                }
            }
            if !any_trajectory {
                out.skipped.push(IntersectionDiagnostic {
                    well: well.id.clone(),
                    bore: String::new(),
                    reason: "missing_trajectory".into(),
                    message: "well has no positioned bore".into(),
                });
            }
        }
        out.hits.sort_by(|a, b| {
            a.well
                .cmp(&b.well)
                .then(a.bore.cmp(&b.bore))
                .then(a.md.total_cmp(&b.md))
        });
        Ok(out)
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
