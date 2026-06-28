//! `Well` → `Sidetrack` → `Trajectory` — the well-geometry hierarchy.
//!
//! A [`Well`] owns one or more named [`Sidetrack`]s (the unnamed `""` is the
//! *main* bore). Each sidetrack owns an ordered list of [`Trajectory`]s with one
//! marked *active*; the newest added becomes active. `Well` and `Sidetrack`
//! delegate position queries (`xyz`/`tvd`/`md_at_tvd`) to the main/active
//! trajectory.
//!
//! Logs and tops (the Phase-4 surface) are intentionally absent here.

use crate::core::trajectory::{Trajectory, TrajectoryInput};
use crate::foundation::{GeoError, Point3, Result};
use indexmap::IndexMap;

/// The label of the main bore.
const MAIN: &str = "";

/// A well: a surface location (`head`), a datum (`kb`), and its sidetracks.
pub struct Well {
    /// Well identifier.
    pub id: String,
    /// Surface location `(x, y)` of the wellhead.
    pub head: (f64, f64),
    /// Kelly-bushing elevation (the measured-depth / TVD datum).
    pub kb: f64,
    /// Bores keyed by label; `""` is the main bore (always present).
    sidetracks: IndexMap<String, Sidetrack>,
}

impl Well {
    /// A new well with an empty main bore. `head` is the wellhead `(x, y)`;
    /// `kb` the kelly-bushing datum.
    pub fn new(id: impl Into<String>, head: (f64, f64), kb: f64) -> Well {
        let mut sidetracks = IndexMap::new();
        sidetracks.insert(MAIN.to_string(), Sidetrack::new(MAIN.to_string(), head, kb));
        Well {
            id: id.into(),
            head,
            kb,
            sidetracks,
        }
    }

    /// The sidetrack with `label`, if it exists.
    pub fn sidetrack(&self, label: &str) -> Option<&Sidetrack> {
        self.sidetracks.get(label)
    }

    /// The sidetrack with `label`, creating an empty one if missing.
    pub fn sidetrack_mut(&mut self, label: &str) -> &mut Sidetrack {
        let (head, kb) = (self.head, self.kb);
        self.sidetracks
            .entry(label.to_string())
            .or_insert_with(|| Sidetrack::new(label.to_string(), head, kb))
    }

    /// The main bore (label `""`), always present.
    pub fn main(&self) -> &Sidetrack {
        self.sidetracks
            .get(MAIN)
            .expect("the main sidetrack is always present")
    }

    /// All sidetracks in insertion order (the main bore first).
    pub fn sidetracks(&self) -> impl Iterator<Item = &Sidetrack> {
        self.sidetracks.values()
    }

    /// Interpolated position at `md` on the main bore's active trajectory.
    pub fn xyz(&self, md: f64) -> Option<Point3> {
        self.main().xyz(md)
    }

    /// TVD at `md` on the main bore's active trajectory.
    pub fn tvd(&self, md: f64) -> Option<f64> {
        self.main().tvd(md)
    }

    /// Measured depth at a TVD on the main bore's active trajectory.
    pub fn md_at_tvd(&self, tvd: f64) -> Option<f64> {
        self.main().md_at_tvd(tvd)
    }
}

/// A single bore: an ordered set of trajectories with one active. Carries the
/// owning well's `head`/`kb` so it can normalize survey input on insert.
pub struct Sidetrack {
    /// The bore label (`""` for the main bore).
    pub label: String,
    head: (f64, f64),
    kb: f64,
    trajectories: Vec<Trajectory>,
    active: usize,
}

impl Sidetrack {
    /// An empty sidetrack carrying its well's `head`/`kb`.
    fn new(label: String, head: (f64, f64), kb: f64) -> Sidetrack {
        Sidetrack {
            label,
            head,
            kb,
            trajectories: Vec::new(),
            active: 0,
        }
    }

    /// Normalize `input` into a trajectory, append it, and make it active.
    /// Returns the new trajectory.
    pub fn add_trajectory(&mut self, input: TrajectoryInput) -> Result<&mut Trajectory> {
        let traj = Trajectory::from_input(input, self.head, self.kb)?;
        self.trajectories.push(traj);
        self.active = self.trajectories.len() - 1;
        Ok(self
            .trajectories
            .last_mut()
            .expect("just pushed a trajectory"))
    }

    /// Select the active trajectory by index. `Err` if out of range.
    pub fn set_active(&mut self, index: usize) -> Result<()> {
        if index >= self.trajectories.len() {
            return Err(GeoError::OutOfRange(format!(
                "trajectory index {index} out of range (have {})",
                self.trajectories.len()
            )));
        }
        self.active = index;
        Ok(())
    }

    /// The active trajectory. Panics if the sidetrack has no trajectory yet —
    /// use [`trajectories`](Self::trajectories) to check first.
    pub fn active(&self) -> &Trajectory {
        self.trajectories
            .get(self.active)
            .expect("active() requires at least one trajectory")
    }

    /// All trajectories in insertion order.
    pub fn trajectories(&self) -> &[Trajectory] {
        &self.trajectories
    }

    /// Interpolated position at `md` on the active trajectory, or `None` when
    /// there is no trajectory or `md` is out of range.
    pub fn xyz(&self, md: f64) -> Option<Point3> {
        self.trajectories.get(self.active).and_then(|t| t.xyz(md))
    }

    /// TVD at `md` on the active trajectory, or `None`.
    pub fn tvd(&self, md: f64) -> Option<f64> {
        self.trajectories.get(self.active).and_then(|t| t.tvd(md))
    }

    /// Measured depth at a TVD on the active trajectory, or `None`.
    pub fn md_at_tvd(&self, tvd: f64) -> Option<f64> {
        self.trajectories
            .get(self.active)
            .and_then(|t| t.md_at_tvd(tvd))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::trajectory::Station;
    use approx::assert_relative_eq;

    fn vertical(md_top: f64, md_bot: f64) -> TrajectoryInput {
        TrajectoryInput::Stations(vec![
            Station::new(md_top, 0.0, 0.0),
            Station::new(md_bot, 0.0, 0.0),
        ])
    }

    #[test]
    fn new_well_has_empty_main() {
        let w = Well::new("15/9-A1", (1000.0, 2000.0), 80.0);
        assert_eq!(w.id, "15/9-A1");
        assert_eq!(w.main().label, "");
        assert!(w.main().trajectories().is_empty());
        assert!(w.xyz(1000.0).is_none());
    }

    #[test]
    fn add_trajectory_makes_newest_active() {
        let mut w = Well::new("w", (0.0, 0.0), 0.0);
        let st = w.sidetrack_mut("");
        st.add_trajectory(vertical(0.0, 1000.0)).unwrap();
        st.add_trajectory(vertical(0.0, 2000.0)).unwrap();
        assert_eq!(st.trajectories().len(), 2);
        // Active is the second (deeper) path.
        assert_eq!(st.active().md_range(), (0.0, 2000.0));
        assert!(st.xyz(1500.0).is_some());
    }

    #[test]
    fn set_active_switches_and_bounds_check() {
        let mut w = Well::new("w", (0.0, 0.0), 0.0);
        let st = w.sidetrack_mut("");
        st.add_trajectory(vertical(0.0, 1000.0)).unwrap();
        st.add_trajectory(vertical(0.0, 2000.0)).unwrap();
        st.set_active(0).unwrap();
        assert_eq!(st.active().md_range(), (0.0, 1000.0));
        assert!(st.set_active(5).is_err());
    }

    #[test]
    fn well_delegates_to_main_active() {
        let mut w = Well::new("w", (500.0, 600.0), 30.0);
        w.sidetrack_mut("")
            .add_trajectory(vertical(0.0, 1000.0))
            .unwrap();
        let p = w.xyz(400.0).unwrap();
        assert_relative_eq!(p.x, 500.0, epsilon = 1e-9);
        assert_relative_eq!(p.y, 600.0, epsilon = 1e-9);
        assert_relative_eq!(p.z, 400.0 - 30.0, epsilon = 1e-9); // tvd = md - kb
        assert_relative_eq!(w.tvd(400.0).unwrap(), 370.0, epsilon = 1e-9);
        assert_relative_eq!(w.md_at_tvd(370.0).unwrap(), 400.0, epsilon = 1e-9);
    }

    #[test]
    fn sidetrack_mut_creates_named_bore_lazily() {
        let mut w = Well::new("w", (0.0, 0.0), 0.0);
        assert!(w.sidetrack("T2").is_none());
        w.sidetrack_mut("T2")
            .add_trajectory(vertical(0.0, 500.0))
            .unwrap();
        assert_eq!(w.sidetrack("T2").unwrap().label, "T2");
        // Two bores now: main + T2.
        assert_eq!(w.sidetracks().count(), 2);
        // The named bore's geometry is independent of the (empty) main.
        assert!(w.sidetrack("T2").unwrap().xyz(250.0).is_some());
        assert!(w.xyz(250.0).is_none()); // main still empty
    }
}
