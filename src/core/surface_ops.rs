//! Surface arithmetic: element-wise math, surface↔surface operations, and the
//! operator overloads that mirror them. Every operation returns a **new**
//! `Surface` (immutable ops); `NaN` (undefined) propagates throughout.

use super::Surface;
use crate::foundation::{GeoError, HasHistory, Result};
use ndarray::Zip;
use std::ops::{Add, Div, Mul, Sub};

impl Surface {
    fn map_unary(&self, op: &str, f: impl Fn(f64) -> f64) -> Surface {
        let mut out = Surface::from_values_unchecked(self.geom.clone(), self.values().mapv(f));
        out.set_history(self.history_with(format!("surface.{op}()")));
        out
    }

    fn map_scalar(&self, op: &str, rhs: f64, f: impl Fn(f64, f64) -> f64) -> Surface {
        let mut out =
            Surface::from_values_unchecked(self.geom.clone(), self.values().mapv(|v| f(v, rhs)));
        out.set_history(self.history_with(format!("surface.{op}_scalar({rhs})")));
        out
    }

    /// Natural log of the primary layer (new surface).
    pub fn ln(&self) -> Surface {
        self.map_unary("ln", f64::ln)
    }
    /// Base-10 log.
    pub fn log10(&self) -> Surface {
        self.map_unary("log10", f64::log10)
    }
    /// Exponential `e^v`.
    pub fn exp(&self) -> Surface {
        self.map_unary("exp", f64::exp)
    }
    /// Square root.
    pub fn sqrt(&self) -> Surface {
        self.map_unary("sqrt", f64::sqrt)
    }
    /// Absolute value.
    pub fn abs(&self) -> Surface {
        self.map_unary("abs", f64::abs)
    }
    /// Raise each node to the power `n`.
    pub fn powf(&self, n: f64) -> Surface {
        self.map_scalar("powf", n, |v, n| v.powf(n))
    }
    /// Clamp each node to a lower bound (`NaN` stays `NaN`).
    pub fn clamp_min(&self, lo: f64) -> Surface {
        self.map_scalar("clamp_min", lo, |v, lo| v.clamp(lo, f64::INFINITY))
    }
    /// Clamp each node to `[lo, hi]` (`NaN` stays `NaN`).
    pub fn clamp(&self, lo: f64, hi: f64) -> Surface {
        let mut out = Surface::from_values_unchecked(
            self.geom.clone(),
            self.values().mapv(|v| v.clamp(lo, hi)),
        );
        out.set_history(self.history_with(format!("surface.clamp({lo}, {hi})")));
        out
    }

    fn binary(&self, other: &Surface, f: impl Fn(f64, f64) -> f64, op: &str) -> Result<Surface> {
        if self.geom != other.geom {
            return Err(GeoError::GeometryMismatch(format!(
                "Surface::{op}: operands have differing geometry — resample first"
            )));
        }
        let values = Zip::from(self.values())
            .and(other.values())
            .map_collect(|&a, &b| f(a, b));
        let mut out = Surface::from_values_unchecked(self.geom.clone(), values);
        let mut history = self.operation_history().clone();
        history.extend_prefixed("rhs", other.operation_history());
        history.push(format!("surface.{op}(surface)"));
        out.set_history(history);
        Ok(out)
    }

    /// Node-wise sum with another surface (equal geometry required).
    pub fn plus(&self, other: &Surface) -> Result<Surface> {
        self.binary(other, |a, b| a + b, "plus")
    }
    /// Node-wise difference (`self - other`).
    pub fn minus(&self, other: &Surface) -> Result<Surface> {
        self.binary(other, |a, b| a - b, "minus")
    }
    /// Node-wise product.
    pub fn times(&self, other: &Surface) -> Result<Surface> {
        self.binary(other, |a, b| a * b, "times")
    }
    /// Node-wise quotient (`self / other`).
    pub fn divided_by(&self, other: &Surface) -> Result<Surface> {
        self.binary(other, |a, b| a / b, "divided_by")
    }

    /// `base - top`, optionally clamped at zero (negative thickness → 0).
    pub fn thickness(top: &Surface, base: &Surface, clamp_zero: bool) -> Result<Surface> {
        let t = base.minus(top)?;
        let mut out = if clamp_zero { t.clamp_min(0.0) } else { t };
        out.record_history(format!("surface.thickness(clamp_zero={clamp_zero})"));
        Ok(out)
    }
}

// Scalar operator overloads → new Surface.
macro_rules! scalar_op {
    ($trait:ident, $method:ident, $f:expr) => {
        impl $trait<f64> for &Surface {
            type Output = Surface;
            fn $method(self, rhs: f64) -> Surface {
                self.map_scalar(stringify!($method), rhs, $f)
            }
        }
    };
}
scalar_op!(Add, add, |a, b| a + b);
scalar_op!(Sub, sub, |a, b| a - b);
scalar_op!(Mul, mul, |a, b| a * b);
scalar_op!(Div, div, |a, b| a / b);

// Surface↔surface operator overloads → Result<Surface> (geometry may mismatch).
macro_rules! surface_op {
    ($trait:ident, $method:ident, $call:ident) => {
        impl $trait<&Surface> for &Surface {
            type Output = Result<Surface>;
            fn $method(self, rhs: &Surface) -> Result<Surface> {
                self.$call(rhs)
            }
        }
    };
}
surface_op!(Add, add, plus);
surface_op!(Sub, sub, minus);
surface_op!(Mul, mul, times);
surface_op!(Div, div, divided_by);

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
            xinc: 1.0,
            yinc: 1.0,
            ncol: 2,
            nrow: 2,
            rotation_deg: 0.0,
            yflip: false,
        }
    }

    fn surf(vals: [f64; 4]) -> Surface {
        // vals in [i,j] order: (0,0),(1,0),(0,1),(1,1)
        let mut v = Array2::zeros((2, 2));
        v[[0, 0]] = vals[0];
        v[[1, 0]] = vals[1];
        v[[0, 1]] = vals[2];
        v[[1, 1]] = vals[3];
        Surface::new(geom(), v).unwrap()
    }

    #[test]
    fn elementwise_math_and_nan_propagation() {
        let s = surf([1.0, 100.0, f64::NAN, 4.0]);
        let l = s.log10();
        assert_relative_eq!(l.values()[[0, 0]], 0.0);
        assert_relative_eq!(l.values()[[1, 0]], 2.0);
        assert!(l.values()[[0, 1]].is_nan()); // NaN propagates
        assert_relative_eq!(s.sqrt().values()[[1, 1]], 2.0);
        // clamp_min keeps NaN as NaN (not lifted to the bound)
        assert!(s.clamp_min(0.0).values()[[0, 1]].is_nan());
        assert_relative_eq!(s.clamp(0.0, 50.0).values()[[1, 0]], 50.0);
    }

    #[test]
    fn scalar_operators() {
        let s = surf([1.0, 2.0, 3.0, 4.0]);
        assert_relative_eq!((&s + 10.0).values()[[0, 0]], 11.0);
        assert_relative_eq!((&s * 2.0).values()[[1, 1]], 8.0);
    }

    #[test]
    fn surface_ops_and_geometry_mismatch() {
        let a = surf([1.0, 2.0, 3.0, 4.0]);
        let b = surf([10.0, 20.0, 30.0, 40.0]);
        assert_relative_eq!(a.plus(&b).unwrap().values()[[0, 0]], 11.0);
        assert_relative_eq!((&b - &a).unwrap().values()[[1, 1]], 36.0);

        // thickness(top, base): base - top, clamped at zero
        let thick = Surface::thickness(&b, &a, true).unwrap(); // a - b is negative → 0
        assert_relative_eq!(thick.values()[[0, 0]], 0.0);

        // differing geometry → GeometryMismatch
        let other = Surface::constant(
            GridGeometry {
                ncol: 3,
                nrow: 3,
                ..geom()
            },
            1.0,
        );
        assert!(a.plus(&other).is_err());
    }
}
