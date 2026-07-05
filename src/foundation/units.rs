//! Length units. petekIO never guesses a unit — it is carried on the project
//! and conversions live here.

/// A length unit. Carried on a `GeoData` project; surfaces/wells inherit it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Unit {
    Feet,
    Metres,
}

impl Unit {
    /// Metres per one of `self`.
    pub fn metres_per_unit(self) -> f64 {
        match self {
            // Single-homed on the family constant (0.3048 exact) so the ft↔m
            // factor has ONE definition across the suite. `area_to_m2` squares
            // this, so the area factor (0.3048²) is single-homed here too.
            Unit::Feet => petektools::units::FT_TO_M,
            Unit::Metres => 1.0,
        }
    }

    /// Convert `value` (in `self` units) to `to` units.
    pub fn convert(self, value: f64, to: Unit) -> f64 {
        value * self.metres_per_unit() / to.metres_per_unit()
    }

    /// Convert a planar area expressed in `self`² (e.g. m² or ft²) to **m²**
    /// (base SI). Backs `ModelInputs::summary.area_m2`. Factor is
    /// `metres_per_unit()²` (0.3048² for feet, 1 for metres).
    pub fn area_to_m2(self, area_in_unit_sq: f64) -> f64 {
        let m = self.metres_per_unit();
        area_in_unit_sq * m * m
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

    #[test]
    fn area_to_m2_hand_calc() {
        // 1 ft² = 0.3048² m² = 0.09290304 m².
        assert_relative_eq!(Unit::Feet.area_to_m2(1.0), 0.092_903_04, epsilon = 1e-9);
        // metres pass through unchanged.
        assert_relative_eq!(Unit::Metres.area_to_m2(4_046.856_422_4), 4_046.856_422_4);
    }
}
