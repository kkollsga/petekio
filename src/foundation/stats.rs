//! `Stats` — the universal aggregation result returned by every reduction
//! (a surface's values, a log over an interval, a point attribute). NaN-skipping
//! throughout; supports optional weighting (by interval length or another curve,
//! e.g. pore-volume-weighted Sw).

/// Summary statistics over a set of defined (non-NaN) values.
///
/// The public fields are the common summaries. The struct also retains the
/// sorted values (and weights, if any) privately so that
/// [`percentile`](Stats::percentile) can return an arbitrary quantile.
#[derive(Debug, Clone, PartialEq)]
pub struct Stats {
    /// Number of defined (non-NaN) values.
    pub count: usize,
    pub mean: f64,
    pub min: f64,
    pub max: f64,
    /// Population standard deviation (weighted, if constructed weighted).
    pub std: f64,
    /// Σ value (unweighted) or Σ wᵢ·vᵢ (weighted).
    pub sum: f64,
    pub p10: f64,
    pub p50: f64,
    pub p90: f64,
    /// Defined values, ascending. Backs `percentile`.
    sorted: Vec<f64>,
    /// Weights aligned to `sorted`; empty when unweighted.
    weights: Vec<f64>,
}

impl Stats {
    /// Summaries over `values`, skipping NaN.
    pub fn of(values: &[f64]) -> Stats {
        let mut v: Vec<f64> = values.iter().copied().filter(|x| !x.is_nan()).collect();
        v.sort_by(f64::total_cmp);
        Stats::from_sorted(v, Vec::new())
    }

    /// Weighted summaries. Pairs with a NaN value/weight or a non-positive
    /// weight are dropped. `mean`/`std`/percentiles are weighted; `min`/`max`
    /// are the plain data extremes.
    pub fn weighted(values: &[f64], weights: &[f64]) -> Stats {
        let mut pairs: Vec<(f64, f64)> = values
            .iter()
            .zip(weights)
            .filter(|(v, w)| !v.is_nan() && !w.is_nan() && **w > 0.0)
            .map(|(v, w)| (*v, *w))
            .collect();
        pairs.sort_by(|a, b| a.0.total_cmp(&b.0));
        let sorted = pairs.iter().map(|p| p.0).collect();
        let w = pairs.iter().map(|p| p.1).collect();
        Stats::from_sorted(sorted, w)
    }

    /// Arbitrary percentile, `p ∈ [0, 1]` (clamped). Linear interpolation for
    /// unweighted stats; cumulative-weight (nearest-rank) for weighted.
    pub fn percentile(&self, p: f64) -> f64 {
        let p = p.clamp(0.0, 1.0);
        if self.weights.is_empty() {
            percentile_linear(&self.sorted, p)
        } else {
            percentile_weighted(&self.sorted, &self.weights, p)
        }
    }

    fn from_sorted(sorted: Vec<f64>, weights: Vec<f64>) -> Stats {
        let count = sorted.len();
        if count == 0 {
            return Stats {
                count: 0,
                mean: f64::NAN,
                min: f64::NAN,
                max: f64::NAN,
                std: f64::NAN,
                sum: 0.0,
                p10: f64::NAN,
                p50: f64::NAN,
                p90: f64::NAN,
                sorted,
                weights,
            };
        }
        let min = sorted[0];
        let max = sorted[count - 1];
        let (sum, mean, std) = if weights.is_empty() {
            let s: f64 = sorted.iter().sum();
            let m = s / count as f64;
            let var = sorted.iter().map(|v| (v - m).powi(2)).sum::<f64>() / count as f64;
            (s, m, var.sqrt())
        } else {
            let wsum: f64 = weights.iter().sum();
            let wv: f64 = sorted.iter().zip(&weights).map(|(v, w)| v * w).sum();
            let m = wv / wsum;
            let var = sorted
                .iter()
                .zip(&weights)
                .map(|(v, w)| w * (v - m).powi(2))
                .sum::<f64>()
                / wsum;
            (wv, m, var.sqrt())
        };
        let pct = |p: f64| {
            if weights.is_empty() {
                percentile_linear(&sorted, p)
            } else {
                percentile_weighted(&sorted, &weights, p)
            }
        };
        let (p10, p50, p90) = (pct(0.10), pct(0.50), pct(0.90));
        Stats {
            count,
            mean,
            min,
            max,
            std,
            sum,
            p10,
            p50,
            p90,
            sorted,
            weights,
        }
    }
}

/// Linear-interpolation percentile (numpy "linear" / method 7) on ascending
/// `sorted`.
fn percentile_linear(sorted: &[f64], p: f64) -> f64 {
    match sorted.len() {
        0 => f64::NAN,
        1 => sorted[0],
        n => {
            let rank = p * (n - 1) as f64;
            let lo = rank.floor() as usize;
            let hi = rank.ceil() as usize;
            let frac = rank - lo as f64;
            sorted[lo] + frac * (sorted[hi] - sorted[lo])
        }
    }
}

/// Cumulative-weight (nearest-rank) percentile on ascending `sorted` with
/// matching `weights`.
fn percentile_weighted(sorted: &[f64], weights: &[f64], p: f64) -> f64 {
    let n = sorted.len();
    if n == 0 {
        return f64::NAN;
    }
    if n == 1 {
        return sorted[0];
    }
    let total: f64 = weights.iter().sum();
    if total <= 0.0 {
        return f64::NAN;
    }
    let target = p * total;
    let mut cum = 0.0;
    for (val, w) in sorted.iter().zip(weights) {
        cum += w;
        if cum >= target {
            return *val;
        }
    }
    sorted[n - 1]
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    #[test]
    fn of_skips_nan_and_matches_hand_calc() {
        let s = Stats::of(&[1.0, 2.0, 3.0, 4.0, f64::NAN]);
        assert_eq!(s.count, 4);
        assert_relative_eq!(s.sum, 10.0);
        assert_relative_eq!(s.mean, 2.5);
        assert_relative_eq!(s.min, 1.0);
        assert_relative_eq!(s.max, 4.0);
        assert_relative_eq!(s.std, 1.25_f64.sqrt()); // population std
        assert_relative_eq!(s.p10, 1.3);
        assert_relative_eq!(s.p50, 2.5);
        assert_relative_eq!(s.p90, 3.7);
        assert_relative_eq!(s.percentile(0.25), 1.75);
    }

    #[test]
    fn all_nan_is_empty() {
        let s = Stats::of(&[f64::NAN, f64::NAN]);
        assert_eq!(s.count, 0);
        assert!(s.mean.is_nan());
        assert_relative_eq!(s.sum, 0.0);
    }

    #[test]
    fn weighted_mean_and_sum() {
        // values 1,2,3 with weights 1,1,2 → wsum=4, Σwv=9, mean=2.25
        let s = Stats::weighted(&[1.0, 2.0, 3.0], &[1.0, 1.0, 2.0]);
        assert_eq!(s.count, 3);
        assert_relative_eq!(s.sum, 9.0);
        assert_relative_eq!(s.mean, 2.25);
        assert_relative_eq!(s.min, 1.0);
        assert_relative_eq!(s.max, 3.0);
    }

    #[test]
    fn weighted_percentile_is_cumulative() {
        // weights put the median mass on 3: total=4, p50 target=2.0 →
        // cum: 1(v=1) <2, 2(v=2) >=2 → 2.0
        let s = Stats::weighted(&[1.0, 2.0, 3.0], &[1.0, 1.0, 2.0]);
        assert_relative_eq!(s.p50, 2.0);
        assert_relative_eq!(s.percentile(0.9), 3.0);
    }
}
