//! `interpret` — petrophysical interpretation: cutoffs → net pay, net-to-gross,
//! reservoir facies flags, and the Leverett J-function. **petekIO owns net_pay
//! derivation** (the agreed ownership boundary: data → model input; consumers do
//! grid-coupled upscaling, never the cutoff interpretation itself).
//!
//! These are pure array kernels (no `Well`/`Trajectory` coupling) so they stay
//! testable against hand calculations and trivially extractable later. The
//! `manager` assembly supplies the per-sample **TVD** depth (via the trajectory)
//! so net thickness is true vertical thickness, not measured-depth.
//!
//! NaN convention: a sample with `NaN` in any required curve cannot be confirmed
//! as pay, so it is treated as non-net.

/// Reservoir cutoffs. A sample is **net pay** iff
/// `phi >= phi_min` AND `sw <= sw_max` AND (`vsh <= vsh_max`, when Vsh is given).
///
/// [`Default`] is a generic clastic starting point — porosity ≥ 8%, water
/// saturation ≤ 50%, shale volume ≤ 50%. Defaulted cutoffs should be flagged
/// [`Assumed`](crate::foundation::Provenance::Assumed) by the caller.
///
/// This is the core value behind the Python `NetSettings` spec: a frozen,
/// value-compared, serialisable cutoff set. [`Display`](std::fmt::Display) prints
/// the one-line cutoff row that backs the spec's `__repr__`.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Cutoffs {
    pub phi_min: f64,
    pub sw_max: f64,
    pub vsh_max: f64,
}

impl Default for Cutoffs {
    fn default() -> Self {
        Self {
            phi_min: 0.08,
            sw_max: 0.5,
            vsh_max: 0.5,
        }
    }
}

impl std::fmt::Display for Cutoffs {
    /// The domain cutoff row: `phi>=0.080  Sw<=0.500  Vsh<=0.500`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "phi>={:.3}  Sw<={:.3}  Vsh<={:.3}",
            self.phi_min, self.sw_max, self.vsh_max
        )
    }
}

/// Per-sample net flag: `true` where the sample passes all cutoffs. `phi` and
/// `sw` are required; pass `vsh = None` to skip the shale cutoff. Any `NaN` in a
/// required (or supplied) curve at a sample yields `false`. Lengths must match
/// `phi`; a shorter `sw`/`vsh` makes trailing samples non-net.
pub fn net_flags(phi: &[f64], sw: &[f64], vsh: Option<&[f64]>, cut: &Cutoffs) -> Vec<bool> {
    (0..phi.len())
        .map(|i| {
            let p = phi[i];
            let s = sw.get(i).copied().unwrap_or(f64::NAN);
            let v = match vsh {
                Some(vsh) => vsh.get(i).copied().unwrap_or(f64::NAN),
                None => 0.0, // no shale cutoff → treat as passing
            };
            p >= cut.phi_min && s <= cut.sw_max && v <= cut.vsh_max && !p.is_nan() && !s.is_nan()
        })
        .collect()
}

/// Per-sample representative thickness (Voronoi/midpoint): each sample owns half
/// the gap to each neighbour, so the thicknesses sum to the total `depth` span.
/// `depth` must be monotonic (TVD for true vertical thickness). Fewer than 2
/// samples → all-zero.
fn representative_thickness(depth: &[f64]) -> Vec<f64> {
    let n = depth.len();
    if n < 2 {
        return vec![0.0; n];
    }
    (0..n)
        .map(|i| {
            let hi = if i + 1 < n { depth[i + 1] } else { depth[i] };
            let lo = if i > 0 { depth[i - 1] } else { depth[i] };
            (hi - lo) / 2.0
        })
        .collect()
}

/// Net pay thickness: the sum of representative thicknesses over net samples.
/// `depth` is per-sample monotonic depth (TVD); `net` is the per-sample flag.
/// Mismatched lengths use the shorter; <2 samples → 0.
pub fn net_pay(depth: &[f64], net: &[bool]) -> f64 {
    let t = representative_thickness(depth);
    t.iter()
        .zip(net)
        .filter(|(_, &is_net)| is_net)
        .map(|(t, _)| *t)
        .sum()
}

/// Net-to-gross over the sampled interval: `net_pay / gross`, where `gross` is
/// the total depth span (`depth.last - depth.first`). Returns 0 when the gross
/// span is non-positive.
pub fn net_to_gross(depth: &[f64], net: &[bool]) -> f64 {
    if depth.len() < 2 {
        return 0.0;
    }
    let gross = depth[depth.len() - 1] - depth[0];
    if gross <= 0.0 {
        return 0.0;
    }
    net_pay(depth, net) / gross
}

/// Leverett J-function (dimensionless): `J = (Pc / ift) * sqrt(perm / phi)`.
/// Units must be consistent so the result is dimensionless — e.g. `Pc` and `ift`
/// in the same pressure/tension units (Pa and N/m), `perm` in m², `phi` a
/// fraction. Returns `NaN` for non-positive `phi`.
pub fn leverett_j(pc: f64, ift: f64, perm: f64, phi: f64) -> f64 {
    if phi <= 0.0 {
        return f64::NAN;
    }
    (pc / ift) * (perm / phi).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    #[test]
    fn default_cutoffs() {
        let c = Cutoffs::default();
        assert_relative_eq!(c.phi_min, 0.08);
        assert_relative_eq!(c.sw_max, 0.5);
        assert_relative_eq!(c.vsh_max, 0.5);
    }

    #[test]
    fn net_flags_apply_all_cutoffs() {
        let phi = [0.20, 0.05, 0.20, 0.20, f64::NAN];
        let sw = [0.30, 0.30, 0.80, 0.30, 0.30];
        let vsh = [0.10, 0.10, 0.10, 0.90, 0.10];
        let net = net_flags(&phi, &sw, Some(&vsh), &Cutoffs::default());
        // 0: pass; 1: low phi; 2: high sw; 3: high vsh; 4: NaN phi
        assert_eq!(net, vec![true, false, false, false, false]);
    }

    #[test]
    fn net_pay_and_ntg_hand_calc() {
        // Regular 10-unit TVD spacing; net the middle three samples.
        let depth = [2400.0, 2410.0, 2420.0, 2430.0, 2440.0];
        let net = [false, true, true, true, false];
        // representative thickness: [5,10,10,10,5]; net = 10+10+10 = 30.
        assert_relative_eq!(net_pay(&depth, &net), 30.0);
        // gross span = 40; NTG = 30/40 = 0.75.
        assert_relative_eq!(net_to_gross(&depth, &net), 0.75);
    }

    #[test]
    fn net_pay_degenerate_single_sample() {
        assert_relative_eq!(net_pay(&[2400.0], &[true]), 0.0);
        assert_relative_eq!(net_to_gross(&[2400.0], &[true]), 0.0);
    }

    #[test]
    fn cutoffs_display_and_serde_roundtrip() {
        let c = Cutoffs {
            phi_min: 0.10,
            sw_max: 0.4,
            vsh_max: 0.3,
        };
        assert_eq!(format!("{c}"), "phi>=0.100  Sw<=0.400  Vsh<=0.300");
        let json = serde_json::to_string(&c).unwrap();
        let back: Cutoffs = serde_json::from_str(&json).unwrap();
        assert_eq!(c, back);
    }

    #[test]
    fn leverett_j_analytic() {
        // Pc=1e5 Pa, ift=0.02 N/m, perm=1e-13 m², phi=0.2.
        // J = (1e5/0.02) * sqrt(1e-13/0.2) = 5e6 * 7.0710678e-7 ≈ 3.5355339.
        assert_relative_eq!(
            leverett_j(1.0e5, 0.02, 1.0e-13, 0.2),
            3.535_533_9,
            epsilon = 1e-6
        );
        assert!(leverett_j(1.0e5, 0.02, 1.0e-13, 0.0).is_nan());
    }
}
