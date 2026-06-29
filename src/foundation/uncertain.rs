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
}
