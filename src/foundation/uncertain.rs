//! Uncertainty & provenance vocabulary for model-ready inputs.
//!
//! Every value petekio hands a consumer as "model-ready" carries not just a
//! number but *how uncertain it is* and *where it came from*. The consumer
//! (e.g. a Monte-Carlo volumetrics engine) **samples and propagates** these; it
//! never re-derives them. See `analysis::model_inputs`.

/// Where a value came from — its hardness. Lets the consumer tell measured data
/// from interpolation, defaults, or expert assumption.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provenance {
    /// Measured / observed directly (a log reading, a survey, a picked top).
    HardData,
    /// Interpolated or gridded between hard data.
    Interpolated,
    /// A default filled in where data was absent.
    Defaulted,
    /// An expert / assumed value with no supporting data.
    Assumed,
}

/// How a value is distributed, for Monte-Carlo propagation. petekio
/// *characterises* this from the data; the consumer *samples* it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Distribution {
    /// A single point value, no spread.
    Deterministic,
    Uniform {
        lo: f64,
        hi: f64,
    },
    Triangular {
        lo: f64,
        mode: f64,
        hi: f64,
    },
    Normal {
        mean: f64,
        std: f64,
    },
    LogNormal {
        mu: f64,
        sigma: f64,
    },
}

/// A model-ready scalar: a point estimate plus the uncertainty and provenance
/// petekio derived from the data.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Uncertain {
    pub value: f64,
    pub distribution: Distribution,
    pub provenance: Provenance,
}

impl Uncertain {
    /// A hard, deterministic datum (point value, measured).
    pub fn hard(value: f64) -> Self {
        Self {
            value,
            distribution: Distribution::Deterministic,
            provenance: Provenance::HardData,
        }
    }

    /// A deterministic value filled in where data was absent.
    pub fn defaulted(value: f64) -> Self {
        Self {
            value,
            distribution: Distribution::Deterministic,
            provenance: Provenance::Defaulted,
        }
    }

    /// A deterministic expert/assumed value with no supporting data.
    pub fn assumed(value: f64) -> Self {
        Self {
            value,
            distribution: Distribution::Deterministic,
            provenance: Provenance::Assumed,
        }
    }

    /// Uniform on `[lo, hi]`; point estimate is the midpoint. Provenance
    /// defaults to [`Interpolated`](Provenance::Interpolated) (a spread implies a
    /// value characterised from data) — override with [`with_provenance`](Self::with_provenance).
    pub fn uniform(lo: f64, hi: f64) -> Self {
        Self::characterised(0.5 * (lo + hi), Distribution::Uniform { lo, hi })
    }

    /// Triangular `(lo, mode, hi)`; point estimate is the mode.
    pub fn triangular(lo: f64, mode: f64, hi: f64) -> Self {
        Self::characterised(mode, Distribution::Triangular { lo, mode, hi })
    }

    /// Normal `(mean, std)`; point estimate is the mean.
    pub fn normal(mean: f64, std: f64) -> Self {
        Self::characterised(mean, Distribution::Normal { mean, std })
    }

    /// Log-normal of the underlying normal `(mu, sigma)`; point estimate is the
    /// median `exp(mu)`.
    pub fn lognormal(mu: f64, sigma: f64) -> Self {
        Self::characterised(mu.exp(), Distribution::LogNormal { mu, sigma })
    }

    /// Characterise an [`Uncertain`] from summary [`Stats`](crate::foundation::Stats):
    /// a [`Normal`](Distribution::Normal) from `mean`/`std`, collapsing to
    /// [`Deterministic`](Distribution::Deterministic) when there is `<2` defined
    /// values or zero spread. Carries the supplied `provenance`.
    pub fn from_stats(stats: &crate::foundation::Stats, provenance: Provenance) -> Self {
        let distribution = if stats.count < 2 || stats.std == 0.0 {
            Distribution::Deterministic
        } else {
            Distribution::Normal {
                mean: stats.mean,
                std: stats.std,
            }
        };
        Self {
            value: stats.mean,
            distribution,
            provenance,
        }
    }

    /// Override the provenance flag (builder style).
    pub fn with_provenance(mut self, provenance: Provenance) -> Self {
        self.provenance = provenance;
        self
    }

    /// A value characterised from data: the given point estimate + distribution,
    /// provenance [`Interpolated`](Provenance::Interpolated) by default.
    fn characterised(value: f64, distribution: Distribution) -> Self {
        Self {
            value,
            distribution,
            provenance: Provenance::Interpolated,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::foundation::Stats;
    use approx::assert_relative_eq;

    #[test]
    fn deterministic_constructors_carry_provenance() {
        assert_eq!(Uncertain::hard(1.0).provenance, Provenance::HardData);
        assert_eq!(Uncertain::defaulted(2.0).provenance, Provenance::Defaulted);
        assert_eq!(Uncertain::assumed(3.0).provenance, Provenance::Assumed);
        assert_eq!(
            Uncertain::hard(1.0).distribution,
            Distribution::Deterministic
        );
    }

    #[test]
    fn distribution_constructors_set_point_estimate() {
        assert_relative_eq!(Uncertain::uniform(2.0, 4.0).value, 3.0); // midpoint
        assert_relative_eq!(Uncertain::triangular(1.0, 2.0, 6.0).value, 2.0); // mode
        assert_relative_eq!(Uncertain::normal(5.0, 1.0).value, 5.0); // mean
        assert_relative_eq!(Uncertain::lognormal(0.0, 0.5).value, 1.0); // exp(0)
                                                                        // a characterised spread defaults to Interpolated provenance
        assert_eq!(
            Uncertain::uniform(2.0, 4.0).provenance,
            Provenance::Interpolated
        );
    }

    #[test]
    fn from_stats_fits_normal_or_collapses() {
        let n = Uncertain::from_stats(&Stats::of(&[1.0, 2.0, 3.0]), Provenance::Interpolated);
        assert_relative_eq!(n.value, 2.0);
        assert!(matches!(n.distribution, Distribution::Normal { .. }));
        // single value → deterministic
        let d = Uncertain::from_stats(&Stats::of(&[7.0]), Provenance::HardData);
        assert_eq!(d.distribution, Distribution::Deterministic);
        assert_relative_eq!(d.value, 7.0);
    }

    #[test]
    fn with_provenance_overrides() {
        let u = Uncertain::uniform(0.0, 1.0).with_provenance(Provenance::Assumed);
        assert_eq!(u.provenance, Provenance::Assumed);
    }
}
