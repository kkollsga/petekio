//! `Log` + `LogView` — a measured-depth-indexed well curve and views over it.
//!
//! A [`Log`] is one curve (e.g. `GR`, `NTG`, `PHIE`) sampled along measured
//! depth: parallel `md`/`values` arrays with `f64::NAN` for undefined samples.
//! It owns its data; reductions happen through a [`LogView`].
//!
//! A [`LogView`] is a *borrowed-or-owned* window onto a log. The common case —
//! an interval clip (see [`crate::core::tops::Interval`]) or the full log — is a
//! zero-copy borrow of a contiguous `md`/`values` sub-slice, which is why
//! `LogView<'a>` carries a lifetime. A [`filter`](LogView::filter) keeps an
//! arbitrary subset, so the view stores [`Cow`] and switches to owned storage
//! when it can no longer be a single contiguous slice.

use crate::foundation::{GeoError, Result, Stats};
use ndarray::Array1;
use std::borrow::Cow;
use std::path::Path;

/// Whether a curve is a continuous log or discrete **core** measurement — so a
/// consumer can include or exclude core data in per-zone aggregation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum LogKind {
    /// A continuous wireline / computed log (the default).
    #[default]
    Log,
    /// Core-derived data (e.g. core porosity/permeability plugs).
    Core,
}

/// One measured-depth-indexed well curve: parallel `md`/`values` with
/// `f64::NAN` for undefined samples. `md` is ascending.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Log {
    /// Curve mnemonic (e.g. `"GR"`, `"NTG"`).
    pub mnemonic: String,
    /// Value unit string from the source (e.g. `"GAPI"`, `"v/v"`).
    pub unit: String,
    /// Whether this is a log or core curve.
    kind: LogKind,
    /// Measured depth of each sample, ascending. Private; reach it via a view.
    md: Array1<f64>,
    /// Sample values aligned to `md`; `NaN` = undefined. Private.
    values: Array1<f64>,
}

impl Log {
    /// A log from parallel `md`/`values`. `Err` if the lengths differ.
    ///
    /// `md` is expected ascending; clipping and `at_md` rely on it.
    pub fn new(
        mnemonic: impl Into<String>,
        unit: impl Into<String>,
        md: Vec<f64>,
        values: Vec<f64>,
    ) -> Result<Log> {
        if md.len() != values.len() {
            return Err(GeoError::Parse(format!(
                "log '{}': md len {} != values len {}",
                mnemonic.into(),
                md.len(),
                values.len()
            )));
        }
        Ok(Log {
            mnemonic: mnemonic.into(),
            unit: unit.into(),
            kind: LogKind::Log,
            md: Array1::from(md),
            values: Array1::from(values),
        })
    }

    /// This curve's kind (log vs core).
    pub fn kind(&self) -> LogKind {
        self.kind
    }

    /// Mark this curve's kind (builder style) — used by the loader to tag core
    /// curves.
    pub fn with_kind(mut self, kind: LogKind) -> Self {
        self.kind = kind;
        self
    }

    /// Load every non-index curve of a LAS file as a [`Log`], each sharing the
    /// file's index (depth) curve as its MD. NULL samples arrive as `f64::NAN`.
    pub fn load_las_all(path: impl AsRef<Path>) -> Result<Vec<Log>> {
        let d = crate::io::las::load(path.as_ref())?;
        let md = d.index;
        d.curves
            .into_iter()
            .map(|c| Log::new(c.mnemonic, c.unit, md.clone(), c.values))
            .collect()
    }

    /// Load a single curve `mnemonic` (case-insensitive) from a LAS file.
    /// `Err(NotFound)` if the file has no such curve.
    pub fn load_las(path: impl AsRef<Path>, mnemonic: &str) -> Result<Log> {
        let d = crate::io::las::load(path.as_ref())?;
        let c = d
            .curves
            .into_iter()
            .find(|c| c.mnemonic.eq_ignore_ascii_case(mnemonic))
            .ok_or_else(|| GeoError::NotFound(format!("LAS curve '{mnemonic}'")))?;
        Log::new(c.mnemonic, c.unit, d.index, c.values)
    }

    /// Number of samples.
    pub fn len(&self) -> usize {
        self.md.len()
    }

    /// Whether the log has no samples.
    pub fn is_empty(&self) -> bool {
        self.md.is_empty()
    }

    /// The measured-depth array as a contiguous slice.
    pub(crate) fn md_slice(&self) -> &[f64] {
        self.md.as_slice().expect("log md is contiguous")
    }

    /// The value array as a contiguous slice.
    pub(crate) fn values_slice(&self) -> &[f64] {
        self.values.as_slice().expect("log values is contiguous")
    }

    /// A borrowed view over the whole log.
    pub fn view(&self) -> LogView<'_> {
        LogView::borrowed(self.md_slice(), self.values_slice())
    }

    /// A borrowed view clipped to the half-open MD window `[top_md, base_md)`.
    /// Returns an empty view when the window selects no samples. Consumed by
    /// [`Interval::log`](crate::core::tops::Interval::log).
    pub(crate) fn clip(&self, top_md: f64, base_md: f64) -> LogView<'_> {
        let md = self.md_slice();
        // md is ascending: first index ≥ top_md … first index ≥ base_md.
        let lo = md.partition_point(|&m| m < top_md);
        let hi = md.partition_point(|&m| m < base_md);
        let hi = hi.max(lo);
        LogView::borrowed(&md[lo..hi], &self.values_slice()[lo..hi])
    }
}

/// A borrowed-or-owned window onto a [`Log`]: a possibly interval-clipped or
/// filtered slice of `md`/`values`. Carries lifetime `'a` so the common
/// (unfiltered) path borrows the log's data with no copy; [`filter`](Self::filter)
/// produces an owned view.
#[derive(Debug, Clone)]
pub struct LogView<'a> {
    md: Cow<'a, [f64]>,
    values: Cow<'a, [f64]>,
}

impl<'a> LogView<'a> {
    /// A zero-copy view borrowing `md`/`values` slices.
    pub(crate) fn borrowed(md: &'a [f64], values: &'a [f64]) -> LogView<'a> {
        LogView {
            md: Cow::Borrowed(md),
            values: Cow::Borrowed(values),
        }
    }

    /// NaN-skipping summary statistics of the view's values.
    pub fn stats(&self) -> Stats {
        Stats::of(&self.values)
    }

    /// Weighted statistics of this view's values using `by`'s values as
    /// weights, aligned **element-wise** (the two views must share sampling —
    /// e.g. two curves clipped to the same interval). Pairs are truncated to
    /// the shorter length; `Stats::weighted` then drops NaN / non-positive
    /// weights. The headline pore-volume-weighted Sw lives here.
    pub fn stats_weighted(&self, by: &LogView) -> Stats {
        let n = self.values.len().min(by.values.len());
        Stats::weighted(&self.values[..n], &by.values[..n])
    }

    /// A new (owned) view keeping only samples whose value satisfies `pred`.
    pub fn filter(&self, pred: impl Fn(f64) -> bool) -> LogView<'a> {
        let mut md = Vec::new();
        let mut values = Vec::new();
        for (&m, &v) in self.md.iter().zip(self.values.iter()) {
            if pred(v) {
                md.push(m);
                values.push(v);
            }
        }
        LogView {
            md: Cow::Owned(md),
            values: Cow::Owned(values),
        }
    }

    /// Linearly interpolated value at measured depth `md`, or `None` outside
    /// the view's MD span (or when the view is empty).
    pub fn at_md(&self, md: f64) -> Option<f64> {
        let m = &self.md;
        let v = &self.values;
        if m.is_empty() || md.is_nan() || md < m[0] || md > m[m.len() - 1] {
            return None;
        }
        // Ascending md: binary-search the bracketing pair. `partition_point`
        // yields the first index with `m[i] >= md`; clamp to `>= 1` so we always
        // have a lower neighbour `m[i - 1]`. Bit-identical to the former linear
        // scan (same bracket, same interpolation order).
        let i = m.partition_point(|&x| x < md).max(1);
        let span = m[i] - m[i - 1];
        if span <= 0.0 {
            return Some(v[i - 1]);
        }
        let t = (md - m[i - 1]) / span;
        Some(v[i - 1] + (v[i] - v[i - 1]) * t)
    }

    /// Resample onto a regular MD grid of spacing `step`, spanning the view's
    /// MD range. Node count is `floor((max-min)/step) + 1`; values come from
    /// [`at_md`](Self::at_md). Returns an owned [`Log`] (mnemonic/unit blank).
    pub fn resample(&self, step: f64) -> Log {
        let m = &self.md;
        let v = &self.values;
        if m.is_empty() || step <= 0.0 {
            return Log {
                mnemonic: String::new(),
                unit: String::new(),
                kind: LogKind::Log,
                md: Array1::from(Vec::new()),
                values: Array1::from(Vec::new()),
            };
        }
        let (lo, hi) = (m[0], m[m.len() - 1]);
        let n = ((hi - lo) / step).floor() as usize + 1;
        let mut md = Vec::with_capacity(n);
        let mut values = Vec::with_capacity(n);
        // Single tandem merge-walk: the output grid is ascending, so the source
        // bracket only advances forward — one pointer `i` across `m` instead of a
        // fresh `at_md` scan per node. Bit-identical to mapping `at_md` over the
        // grid (same bracket `i`, same `span<=0` guard, same interpolation).
        let last = m.len() - 1;
        let mut i = 1usize;
        for k in 0..n {
            let d = lo + step * k as f64;
            md.push(d);
            // `d >= lo` always; guard the upper edge exactly as `at_md` does.
            if d > m[last] {
                values.push(f64::NAN);
                continue;
            }
            while i < last && m[i] < d {
                i += 1;
            }
            let span = m[i] - m[i - 1];
            if span <= 0.0 {
                values.push(v[i - 1]);
            } else {
                let t = (d - m[i - 1]) / span;
                values.push(v[i - 1] + (v[i] - v[i - 1]) * t);
            }
        }
        Log {
            mnemonic: String::new(),
            unit: String::new(),
            kind: LogKind::Log,
            md: Array1::from(md),
            values: Array1::from(values),
        }
    }

    /// Resample this view onto **arbitrary ascending** `targets` via a single
    /// tandem merge-walk — one forward pointer across the view's MD instead of a
    /// fresh [`at_md`](Self::at_md) binary-search per target. Bit-identical to
    /// mapping `at_md` over `targets` (same bracket, same `span <= 0` guard, same
    /// interpolation order); a `NaN`, below-span, or above-span target yields
    /// `NaN`, and an empty view yields all-`NaN`. `targets` **must be ascending**
    /// (the pointer never rewinds); the well conditioning path (three cutoff
    /// curves resampled onto each zone's shared MD grid) is the one home for this
    /// O(n+k) walk, replacing a per-sample O(k·n) `at_md` sweep.
    pub fn resample_onto(&self, targets: &[f64]) -> Vec<f64> {
        let m = &self.md;
        let v = &self.values;
        if m.is_empty() {
            return vec![f64::NAN; targets.len()];
        }
        let last = m.len() - 1;
        let mut i = 1usize;
        let mut out = Vec::with_capacity(targets.len());
        for &d in targets {
            if d.is_nan() || d < m[0] || d > m[last] {
                out.push(f64::NAN);
                continue;
            }
            while i < last && m[i] < d {
                i += 1;
            }
            let span = m[i] - m[i - 1];
            if span <= 0.0 {
                out.push(v[i - 1]);
            } else {
                let t = (d - m[i - 1]) / span;
                out.push(v[i - 1] + (v[i] - v[i - 1]) * t);
            }
        }
        out
    }

    /// The view's values.
    pub fn values(&self) -> &[f64] {
        &self.values
    }

    /// The view's measured depths.
    pub fn md(&self) -> &[f64] {
        &self.md
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    fn log() -> Log {
        // md 100..=140 step 10; one NaN sample.
        Log::new(
            "NTG",
            "v/v",
            vec![100.0, 110.0, 120.0, 130.0, 140.0],
            vec![0.2, 0.4, f64::NAN, 0.8, 1.0],
        )
        .unwrap()
    }

    #[test]
    fn new_rejects_mismatched_lengths() {
        assert!(Log::new("X", "u", vec![1.0, 2.0], vec![1.0]).is_err());
    }

    #[test]
    fn view_stats_skip_nan() {
        let l = log();
        let s = l.view().stats();
        assert_eq!(s.count, 4); // NaN dropped
        assert_relative_eq!(s.sum, 0.2 + 0.4 + 0.8 + 1.0);
        assert_relative_eq!(s.mean, 2.4 / 4.0);
        assert_relative_eq!(s.min, 0.2);
        assert_relative_eq!(s.max, 1.0);
    }

    #[test]
    fn clip_selects_half_open_window() {
        let l = log();
        // [110, 130): samples at 110, 120 → values 0.4, NaN.
        let v = l.clip(110.0, 130.0);
        assert_eq!(v.md(), &[110.0, 120.0]);
        assert_eq!(v.values()[0], 0.4);
        assert!(v.values()[1].is_nan());
        // Stats over the clip skip the NaN → one value.
        assert_eq!(v.stats().count, 1);
    }

    #[test]
    fn clip_to_td_includes_last_sample() {
        let l = log();
        // base beyond the deepest sample → includes 140.0.
        let v = l.clip(130.0, 1e9);
        assert_eq!(v.md(), &[130.0, 140.0]);
    }

    #[test]
    fn filter_keeps_predicate_subset() {
        let l = log();
        let v = l.view().filter(|x| x >= 0.5);
        assert_eq!(v.values(), &[0.8, 1.0]);
        assert_eq!(v.md(), &[130.0, 140.0]);
    }

    #[test]
    fn at_md_interpolates_and_bounds() {
        let l = Log::new("X", "u", vec![100.0, 200.0], vec![0.0, 10.0]).unwrap();
        let v = l.view();
        assert_relative_eq!(v.at_md(150.0).unwrap(), 5.0);
        assert_relative_eq!(v.at_md(100.0).unwrap(), 0.0);
        assert_relative_eq!(v.at_md(200.0).unwrap(), 10.0);
        assert!(v.at_md(99.0).is_none());
        assert!(v.at_md(201.0).is_none());
    }

    #[test]
    fn resample_node_count_and_values() {
        let l = Log::new("X", "u", vec![100.0, 200.0], vec![0.0, 10.0]).unwrap();
        let r = l.view().resample(25.0);
        // 100,125,150,175,200 → 5 nodes
        assert_eq!(r.len(), 5);
        assert_eq!(r.md_slice(), &[100.0, 125.0, 150.0, 175.0, 200.0]);
        assert_relative_eq!(r.values_slice()[1], 2.5);
        assert_relative_eq!(r.values_slice()[4], 10.0);
    }

    /// A naive linear-scan reference for `at_md` — the pre-optimization body,
    /// kept in the test to golden the binary-search rewrite bit-for-bit.
    fn at_md_linear(m: &[f64], v: &[f64], md: f64) -> Option<f64> {
        if m.is_empty() || md.is_nan() || md < m[0] || md > m[m.len() - 1] {
            return None;
        }
        for i in 1..m.len() {
            if md <= m[i] {
                let span = m[i] - m[i - 1];
                if span <= 0.0 {
                    return Some(v[i - 1]);
                }
                let t = (md - m[i - 1]) / span;
                return Some(v[i - 1] + (v[i] - v[i - 1]) * t);
            }
        }
        Some(v[m.len() - 1])
    }

    #[test]
    fn at_md_binary_search_matches_linear_scan_bit_for_bit() {
        // Irregular ascending md (variable spacing + one flat step), NaN values.
        let md: Vec<f64> = (0..500)
            .map(|i| 1000.0 + (i as f64).powf(1.3) * 0.4)
            .collect();
        let mut md = md;
        md[200] = md[199]; // flat step → span == 0 branch
        let values: Vec<f64> = (0..500)
            .map(|i| {
                if i % 13 == 0 {
                    f64::NAN
                } else {
                    (i as f64 * 0.017).sin()
                }
            })
            .collect();
        let l = Log::new("X", "u", md.clone(), values.clone()).unwrap();
        let view = l.view();
        // Sweep query depths on-node, mid-bracket, and just outside both ends.
        for k in 0..2000 {
            let d = 995.0 + k as f64 * 0.65;
            let got = view.at_md(d);
            let want = at_md_linear(&md, &values, d);
            match (got, want) {
                (Some(a), Some(b)) => assert_eq!(a.to_bits(), b.to_bits(), "at_md({d})"),
                (None, None) => {}
                _ => panic!("at_md({d}) None/Some mismatch: {got:?} vs {want:?}"),
            }
        }
    }

    #[test]
    fn resample_merge_walk_matches_per_node_at_md_bit_for_bit() {
        let md: Vec<f64> = (0..400)
            .map(|i| 500.0 + (i as f64).powf(1.2) * 0.5)
            .collect();
        let values: Vec<f64> = (0..400)
            .map(|i| {
                if i % 11 == 0 {
                    f64::NAN
                } else {
                    0.2 + (i as f64 * 0.03).cos()
                }
            })
            .collect();
        let l = Log::new("X", "u", md.clone(), values.clone()).unwrap();
        let view = l.view();
        for &step in &[0.1_f64, 0.37, 1.0, 2.5, 7.3] {
            let r = view.resample(step);
            // Reference: independent per-node linear-scan at_md over the same grid.
            let lo = md[0];
            let hi = md[md.len() - 1];
            let n = ((hi - lo) / step).floor() as usize + 1;
            assert_eq!(r.len(), n, "node count for step {step}");
            for k in 0..n {
                let d = lo + step * k as f64;
                let want = at_md_linear(&md, &values, d).unwrap_or(f64::NAN);
                assert_eq!(
                    r.values_slice()[k].to_bits(),
                    want.to_bits(),
                    "resample(step={step}) node {k} @ md {d}"
                );
                assert_eq!(r.md_slice()[k], d);
            }
        }
    }

    #[test]
    fn resample_onto_matches_per_target_at_md_bit_for_bit() {
        // Irregular ascending md (variable spacing + one flat step), NaN values.
        let mut md: Vec<f64> = (0..300)
            .map(|i| 800.0 + (i as f64).powf(1.25) * 0.4)
            .collect();
        md[150] = md[149]; // flat step → span == 0 branch
        let values: Vec<f64> = (0..300)
            .map(|i| {
                if i % 17 == 0 {
                    f64::NAN
                } else {
                    (i as f64 * 0.02).cos()
                }
            })
            .collect();
        let l = Log::new("X", "u", md.clone(), values.clone()).unwrap();
        let view = l.view();
        // Ascending targets: on-node, mid-bracket, and outside both ends.
        let targets: Vec<f64> = (0..1500).map(|k| 790.0 + k as f64 * 0.9).collect();
        let got = view.resample_onto(&targets);
        assert_eq!(got.len(), targets.len());
        for (k, &d) in targets.iter().enumerate() {
            let want = view.at_md(d).unwrap_or(f64::NAN);
            assert_eq!(got[k].to_bits(), want.to_bits(), "resample_onto @ {d}");
        }
        // NaN target and empty view.
        let with_nan = view.resample_onto(&[md[0], f64::NAN, md[10]]);
        assert!(with_nan[1].is_nan());
        let empty = Log::new("E", "u", vec![], vec![]).unwrap();
        assert!(empty
            .view()
            .resample_onto(&[1.0, 2.0])
            .iter()
            .all(|x| x.is_nan()));
    }

    #[test]
    fn stats_weighted_pv_vs_hand_calc() {
        // Sw values weighted by pore volume (PV).
        let sw = Log::new("SW", "v/v", vec![10.0, 20.0, 30.0], vec![0.2, 0.5, 0.8]).unwrap();
        let pv = Log::new("PV", "m3", vec![10.0, 20.0, 30.0], vec![1.0, 1.0, 2.0]).unwrap();
        // Σwv = 0.2 + 0.5 + 1.6 = 2.3 ; Σw = 4 → mean 0.575
        let s = sw.view().stats_weighted(&pv.view());
        assert_relative_eq!(s.sum, 2.3);
        assert_relative_eq!(s.mean, 0.575);
        assert_eq!(s.count, 3);
    }
}
