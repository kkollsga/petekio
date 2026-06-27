//! Length units. petekIO never guesses a unit — it is carried on the project
//! and conversions live here.

/// A length unit. Carried on a `GeoData` project; surfaces/wells inherit it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    Feet,
    Metres,
}

impl Unit {
    /// Metres per one of `self`.
    pub fn metres_per_unit(self) -> f64 {
        match self {
            Unit::Feet => 0.3048,
            Unit::Metres => 1.0,
        }
    }

    /// Convert `value` (in `self` units) to `to` units.
    pub fn convert(self, value: f64, to: Unit) -> f64 {
        value * self.metres_per_unit() / to.metres_per_unit()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    #[test]
    fn feet_to_metres_roundtrip() {
        assert_relative_eq!(Unit::Feet.convert(100.0, Unit::Metres), 30.48);
        assert_relative_eq!(Unit::Metres.convert(30.48, Unit::Feet), 100.0);
        assert_relative_eq!(Unit::Feet.convert(42.0, Unit::Feet), 42.0);
    }
}
