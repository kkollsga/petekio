//! `validate` — bounds / validity-range checks on normalized curves. The second
//! half of the path to [`ModelInputs`](super::model_inputs::ModelInputs): after
//! [`normalize`](super::normalize) has canonicalised mnemonics and units, this
//! pass rejects physically-impossible samples (a porosity of 1.4, a negative Sw)
//! by setting them to `NaN` — the project-wide "undefined" convention, so
//! downstream stats skip them rather than poisoning a mean.
//!
//! **Provenance** is *not* assigned here. It is carried via
//! [`Provenance`](crate::foundation::Provenance) at the point each value is
//! derived: a measured log sample is [`HardData`](crate::foundation::Provenance::HardData),
//! a gridded/interpolated value is [`Interpolated`](crate::foundation::Provenance::Interpolated),
//! and a filled cutoff/default is [`Defaulted`](crate::foundation::Provenance::Defaulted) /
//! [`Assumed`](crate::foundation::Provenance::Assumed) — see `interpret` + the
//! `manager` assembly. Validation only governs *which samples are defined*.
//!
//! Open/closed: ranges are extended by adding a row to [`validity_range`]; an
//! unranged mnemonic accepts any finite value.

/// Inclusive physical validity range `(lo, hi)` for a canonical mnemonic
/// (post-[`normalize`](super::normalize)), or `None` when no range is registered
/// (then any finite value is accepted). Match is case-insensitive.
pub fn validity_range(canonical_mnemonic: &str) -> Option<(f64, f64)> {
    let key = canonical_mnemonic.trim().to_ascii_uppercase();
    let range = match key.as_str() {
        "PHIE" | "PHIT" | "NPHI" => (0.0, 1.0), // porosity fraction
        "SW" => (0.0, 1.0),                     // water saturation fraction
        "VSH" => (0.0, 1.0),                    // shale volume fraction
        "NTG" => (0.0, 1.0),                    // net-to-gross fraction
        "GR" => (0.0, 250.0),                   // gamma ray, API
        "RHOB" => (1.0, 3.5),                   // bulk density, g/cc
        "DT" => (40.0, 200.0),                  // interval transit time, us/ft
        "PERM" => (0.0, 1.0e6),                 // permeability, mD (non-negative)
        "RT" => (0.0, 1.0e6),                   // deep resistivity, ohm·m (non-negative)
        _ => return None,
    };
    Some(range)
}

/// True if `value` is within the mnemonic's validity range. `NaN` is never in
/// range; an unranged mnemonic accepts any finite value.
pub fn in_range(canonical_mnemonic: &str, value: f64) -> bool {
    if value.is_nan() {
        return false;
    }
    match validity_range(canonical_mnemonic) {
        Some((lo, hi)) => (lo..=hi).contains(&value),
        None => value.is_finite(),
    }
}

/// Mask out-of-range samples to `NaN` in place; returns the count rejected.
/// Already-`NaN` samples are left untouched (they are not counted). An unranged
/// mnemonic is a no-op (returns 0).
pub fn mask_out_of_range(canonical_mnemonic: &str, values: &mut [f64]) -> usize {
    let Some((lo, hi)) = validity_range(canonical_mnemonic) else {
        return 0;
    };
    let mut rejected = 0;
    for v in values.iter_mut() {
        if v.is_nan() {
            continue;
        }
        if !(lo..=hi).contains(v) {
            *v = f64::NAN;
            rejected += 1;
        }
    }
    rejected
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges_registered_and_unranged() {
        assert_eq!(validity_range("PHIE"), Some((0.0, 1.0)));
        assert_eq!(validity_range("sw"), Some((0.0, 1.0))); // case-insensitive
        assert_eq!(validity_range("CUSTOM"), None);
    }

    #[test]
    fn in_range_rejects_nan_and_out_of_bounds() {
        assert!(in_range("PHIE", 0.25));
        assert!(in_range("PHIE", 0.0)); // inclusive
        assert!(!in_range("PHIE", 1.4));
        assert!(!in_range("PHIE", f64::NAN));
        assert!(in_range("CUSTOM", 999.0)); // unranged accepts finite
        assert!(!in_range("CUSTOM", f64::INFINITY));
    }

    #[test]
    fn mask_sets_out_of_range_to_nan() {
        let mut v = vec![0.1, 1.4, -0.2, 0.3, f64::NAN];
        let rejected = mask_out_of_range("PHIE", &mut v);
        assert_eq!(rejected, 2); // 1.4 and -0.2
        assert_eq!(v[0], 0.1);
        assert!(v[1].is_nan());
        assert!(v[2].is_nan());
        assert_eq!(v[3], 0.3);
        assert!(v[4].is_nan()); // pre-existing NaN, not counted
    }

    #[test]
    fn mask_unranged_is_noop() {
        let mut v = vec![999.0, -5.0];
        assert_eq!(mask_out_of_range("CUSTOM", &mut v), 0);
        assert_eq!(v, vec![999.0, -5.0]);
    }
}
