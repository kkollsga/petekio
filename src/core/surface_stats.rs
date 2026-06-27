//! Surface statistics & volumetrics: `stats`, `area_below`/`area_above`,
//! `volume_between`, and the `hypsometry` (area-vs-depth) curve. Cell area is
//! `xinc · yinc` — rotation is an isometry, so it does not change area.

use super::Surface;
use crate::foundation::{GeoError, Result, Stats};

impl Surface {
    /// The cell (node) area, `xinc · yinc`.
    fn cell_area(&self) -> f64 {
        self.geom.xinc * self.geom.yinc
    }

    /// Summary statistics over the defined (non-NaN) nodes of the primary layer.
    pub fn stats(&self) -> Stats {
        let values: Vec<f64> = self.values().iter().copied().collect();
        Stats::of(&values)
    }

    /// Areal extent of nodes whose value is `≤ depth` (each defined node owns
    /// one cell of area `xinc · yinc`). The GRV-style query.
    pub fn area_below(&self, depth: f64) -> f64 {
        let n = self
            .values()
            .iter()
            .copied()
            .filter(|&v| !v.is_nan() && v <= depth)
            .count();
        n as f64 * self.cell_area()
    }

    /// Areal extent of nodes whose value is `≥ depth`.
    pub fn area_above(&self, depth: f64) -> f64 {
        let n = self
            .values()
            .iter()
            .copied()
            .filter(|&v| !v.is_nan() && v >= depth)
            .count();
        n as f64 * self.cell_area()
    }

    /// Volume between this surface and `base` (equal geometry required):
    /// `Σ |selfᵢ − baseᵢ| · cell_area` over nodes defined in both.
    pub fn volume_between(&self, base: &Surface) -> Result<f64> {
        if self.geom != base.geom {
            return Err(GeoError::GeometryMismatch(
                "Surface::volume_between: operands have differing geometry — resample first".into(),
            ));
        }
        let cell = self.cell_area();
        let vol = self
            .values()
            .iter()
            .zip(base.values().iter())
            .filter(|(a, b)| !a.is_nan() && !b.is_nan())
            .map(|(a, b)| (a - b).abs() * cell)
            .sum();
        Ok(vol)
    }

    /// The hypsometric curve: `(depth, area)` points, where `area` is the areal
    /// extent of nodes `≤ depth`. Ascending in both depth and area.
    pub fn hypsometry(&self) -> Vec<(f64, f64)> {
        let cell = self.cell_area();
        let mut vals: Vec<f64> = self
            .values()
            .iter()
            .copied()
            .filter(|v| !v.is_nan())
            .collect();
        vals.sort_by(f64::total_cmp);
        vals.iter()
            .enumerate()
            .map(|(k, &v)| (v, (k + 1) as f64 * cell))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::foundation::GridGeometry;
    use approx::assert_relative_eq;
    use ndarray::Array2;

    fn geom() -> GridGeometry {
        GridGeometry {
            xori: 0.0,
            yori: 0.0,
            xinc: 10.0,
            yinc: 10.0,
            ncol: 2,
            nrow: 2,
            rotation_deg: 0.0,
            yflip: false,
        }
    }

    /// 2×2 ramp 0/10/20/30, cell area = 100.
    fn ramp() -> Surface {
        let mut v = Array2::zeros((2, 2));
        v[[0, 0]] = 0.0;
        v[[1, 0]] = 10.0;
        v[[0, 1]] = 20.0;
        v[[1, 1]] = 30.0;
        Surface::new(geom(), v).unwrap()
    }

    #[test]
    fn stats_over_defined_nodes() {
        let s = ramp().stats();
        assert_eq!(s.count, 4);
        assert_relative_eq!(s.mean, 15.0);
        assert_relative_eq!(s.min, 0.0);
        assert_relative_eq!(s.max, 30.0);
    }

    #[test]
    fn area_below_above_analytic() {
        let s = ramp(); // cell area 100
        assert_relative_eq!(s.area_below(15.0), 200.0); // {0,10}
        assert_relative_eq!(s.area_below(25.0), 300.0); // {0,10,20}
        assert_relative_eq!(s.area_below(100.0), 400.0); // all
        assert_relative_eq!(s.area_above(15.0), 200.0); // {20,30}
    }

    #[test]
    fn volume_between_hand_calc() {
        let s = ramp();
        let base = Surface::constant(geom(), -5.0);
        // |v-(-5)| = 5,15,25,35 → sum 80 × cell 100 = 8000
        assert_relative_eq!(s.volume_between(&base).unwrap(), 8000.0);
    }

    #[test]
    fn hypsometry_is_monotonic() {
        let h = ramp().hypsometry();
        assert_eq!(h.len(), 4);
        assert_relative_eq!(h.last().unwrap().1, 400.0);
        for w in h.windows(2) {
            assert!(w[1].0 >= w[0].0); // depth ascending
            assert!(w[1].1 >= w[0].1); // area ascending
        }
    }
}
