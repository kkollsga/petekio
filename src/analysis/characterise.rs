//! `characterise` — turn a sample set into an [`Uncertain`]: fit a
//! [`Distribution`](crate::foundation::Distribution) and attach a
//! [`Provenance`](crate::foundation::Provenance). The last pass before assembly —
//! petekIO *characterises* the spread from data; the consumer *samples and
//! propagates* it (it never re-derives the distribution).
//!
//! All fits are NaN-skipping (via [`Stats`]) and collapse to
//! [`Deterministic`](crate::foundation::Distribution::Deterministic) when there
//! are fewer than two defined values — a lone reading has no spread to fit.

use crate::foundation::{Provenance, Stats, Uncertain};

/// Which distribution family to fit when characterising a sample set.
///
/// - [`Normal`](DistributionShape::Normal) — symmetric data (e.g. porosity).
/// - [`Triangular`](DistributionShape::Triangular) — the reservoir-engineering
///   P10/P50/P90 idiom: `lo = p10`, `mode = p50`, `hi = p90`.
/// - [`LogNormal`](DistributionShape::LogNormal) — right-skewed, positive data
///   (e.g. permeability); fitted on the natural log of the positive samples.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DistributionShape {
    Normal,
    Triangular,
    LogNormal,
}

/// Characterise an [`Uncertain`] from `values` using `shape`, carrying
/// `provenance`. NaN values are skipped; `<2` defined values → deterministic at
/// the mean. [`LogNormal`](DistributionShape::LogNormal) is fitted on positive
/// samples only and falls back to deterministic if fewer than two remain.
pub fn characterise(values: &[f64], shape: DistributionShape, provenance: Provenance) -> Uncertain {
    let stats = Stats::of(values);
    if stats.count < 2 {
        return Uncertain {
            value: stats.mean,
            distribution: crate::foundation::Distribution::Deterministic,
            provenance,
        };
    }
    match shape {
        DistributionShape::Normal => Uncertain::from_stats(&stats, provenance),
        DistributionShape::Triangular => {
            Uncertain::triangular(stats.p10, stats.p50, stats.p90).with_provenance(provenance)
        }
        DistributionShape::LogNormal => {
            let logs: Vec<f64> = values
                .iter()
                .filter(|v| v.is_finite() && **v > 0.0)
                .map(|v| v.ln())
                .collect();
            if logs.len() < 2 {
                return Uncertain::defaulted(stats.mean).with_provenance(provenance);
            }
            let ls = Stats::of(&logs);
            Uncertain::lognormal(ls.mean, ls.std).with_provenance(provenance)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::foundation::Distribution;
    use approx::assert_relative_eq;

    #[test]
    fn normal_fit_carries_mean_and_provenance() {
        let u = characterise(
            &[1.0, 2.0, 3.0],
            DistributionShape::Normal,
            Provenance::Interpolated,
        );
        assert_relative_eq!(u.value, 2.0);
        assert!(matches!(u.distribution, Distribution::Normal { .. }));
        assert_eq!(u.provenance, Provenance::Interpolated);
    }

    #[test]
    fn triangular_uses_p10_p50_p90() {
        let u = characterise(
            &[1.0, 2.0, 3.0, 4.0, 5.0],
            DistributionShape::Triangular,
            Provenance::HardData,
        );
        // Stats linear percentiles over 5 sorted: p10=1.4, p50=3.0, p90=4.6.
        assert_relative_eq!(u.value, 3.0); // mode = p50
        match u.distribution {
            Distribution::Triangular { lo, mode, hi } => {
                assert_relative_eq!(lo, 1.4, epsilon = 1e-9);
                assert_relative_eq!(mode, 3.0, epsilon = 1e-9);
                assert_relative_eq!(hi, 4.6, epsilon = 1e-9);
            }
            other => panic!("expected Triangular, got {other:?}"),
        }
        assert_eq!(u.provenance, Provenance::HardData);
    }

    #[test]
    fn lognormal_fits_on_log_and_returns_geometric_mean() {
        // ln of [1, e, e²] = [0, 1, 2]; mu = 1 → value = exp(1) = e.
        let u = characterise(
            &[1.0, std::f64::consts::E, std::f64::consts::E.powi(2)],
            DistributionShape::LogNormal,
            Provenance::Interpolated,
        );
        assert_relative_eq!(u.value, std::f64::consts::E, epsilon = 1e-9);
        match u.distribution {
            Distribution::LogNormal { mu, .. } => assert_relative_eq!(mu, 1.0, epsilon = 1e-9),
            other => panic!("expected LogNormal, got {other:?}"),
        }
    }

    #[test]
    fn collapses_to_deterministic_below_two() {
        let u = characterise(&[7.0], DistributionShape::Normal, Provenance::HardData);
        assert_eq!(u.distribution, Distribution::Deterministic);
        assert_relative_eq!(u.value, 7.0);
    }
}
