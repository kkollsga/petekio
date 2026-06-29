//! `Trajectory` — build a positioned well path from a directional survey and
//! query it (MD → position / TVD). Mirrors `petekio::Trajectory`. Unlike the
//! `Well` views, this is a standalone owned object: construct it directly from
//! stations, no `GeoData` project required.

use crate::to_pyerr;
use petekio::{Station, Trajectory as RsTrajectory, TrajectoryInput};
use pyo3::prelude::*;

/// A minimum-curvature well path positioned from a wellhead + KB datum.
#[pyclass(name = "Trajectory")]
pub struct Trajectory {
    inner: RsTrajectory,
}

#[pymethods]
impl Trajectory {
    /// Build from a directional survey: `stations` is a list of
    /// `(md, inc_deg, azi_deg)` (inclination from vertical, azimuth clockwise
    /// from North), positioned from `head = (x, y)` and kelly-bushing `kb` via
    /// the minimum-curvature method. `z` is subsea TVD (positive down); a
    /// vertical hole satisfies `tvd(md) = md - kb`.
    #[staticmethod]
    #[pyo3(signature = (stations, head, kb))]
    fn from_stations(
        stations: Vec<(f64, f64, f64)>,
        head: (f64, f64),
        kb: f64,
    ) -> PyResult<Trajectory> {
        let stations = stations
            .into_iter()
            .map(|(md, inc, azi)| Station::new(md, inc, azi))
            .collect();
        let inner = RsTrajectory::from_input(TrajectoryInput::Stations(stations), head, kb)
            .map_err(to_pyerr)?;
        Ok(Trajectory { inner })
    }

    /// `(min, max)` measured-depth span of the path.
    fn md_range(&self) -> (f64, f64) {
        self.inner.md_range()
    }

    /// Interpolated world position `(x, y, z=TVDSS)` at `md`, or `None` outside
    /// the MD range.
    fn xyz(&self, md: f64) -> Option<(f64, f64, f64)> {
        self.inner.xyz(md).map(|p| (p.x, p.y, p.z))
    }

    /// Subsea true vertical depth at `md`, or `None` outside the MD range.
    fn tvd(&self, md: f64) -> Option<f64> {
        self.inner.tvd(md)
    }

    /// Measured depth at a given TVD (shallowest crossing), or `None`.
    fn md_at_tvd(&self, tvd: f64) -> Option<f64> {
        self.inner.md_at_tvd(tvd)
    }

    fn __repr__(&self) -> String {
        let (lo, hi) = self.inner.md_range();
        format!("Trajectory(md {lo:.1}..{hi:.1})")
    }
}
