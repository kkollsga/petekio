//! `Well` → `Sidetrack` → `Trajectory` — the well-geometry hierarchy.
//!
//! A [`Well`] owns one or more named [`Sidetrack`]s (the unnamed `""` is the
//! *main* bore). Each sidetrack owns an ordered list of [`Trajectory`]s with one
//! marked *active*; the newest added becomes active. `Well` and `Sidetrack`
//! delegate position queries (`xyz`/`tvd`/`md_at_tvd`) to the main/active
//! trajectory.
//!
//! Each sidetrack also owns its [`Log`]s and formation [`Top`]s: `add_log`/
//! `add_tops` ingest them, `log` returns a full-curve [`LogView`], and `top`
//! resolves a marker into the [`Interval`] it names (base = the next top's MD by
//! sorted MD, else the active trajectory's total depth). `Well` delegates `top`/
//! `log` to the main bore.

use crate::core::log::{Log, LogView};
use crate::core::tops::{Interval, Top};
use crate::core::trajectory::{Trajectory, TrajectoryInput};
use crate::foundation::{GeoError, Point3, Result, Stats};
use indexmap::IndexMap;
use std::collections::HashMap;

/// The label of the main bore.
const MAIN: &str = "";

/// A well: a surface location (`head`), a datum (`kb`), and its sidetracks.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct Well {
    /// Well identifier.
    pub id: String,
    /// Surface location `(x, y)` of the wellhead.
    pub head: (f64, f64),
    /// Kelly-bushing elevation (the measured-depth / TVD datum).
    pub kb: f64,
    /// Coordinate reference system label (e.g. `"ED50 / UTM zone 31N"`), if
    /// known. Recorded for provenance; petekIO never reprojects.
    crs: Option<String>,
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
            crs: None,
            sidetracks,
        }
    }

    /// The coordinate reference system label, if recorded.
    pub fn crs(&self) -> Option<&str> {
        self.crs.as_deref()
    }

    /// Record the coordinate reference system label (provenance only).
    pub fn set_crs(&mut self, crs: impl Into<String>) {
        self.crs = Some(crs.into());
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

    /// The interval named by top `name` on the main bore, or `None`.
    pub fn top(&self, name: &str) -> Option<Interval<'_>> {
        self.main().top(name)
    }

    /// A full-curve view of log `mnemonic` on the main bore, or `None`.
    pub fn log(&self, mnemonic: &str) -> Option<LogView<'_>> {
        self.main().log(mnemonic)
    }

    /// All logs on the main bore, in insertion order.
    pub fn logs(&self) -> impl Iterator<Item = &Log> {
        self.main().logs()
    }

    /// The mnemonics of all main-bore logs, in insertion order.
    pub fn mnemonics(&self) -> Vec<&str> {
        self.main().logs().map(|l| l.mnemonic.as_str()).collect()
    }

    /// Every formation zone on the main bore (see [`Sidetrack::zones`]).
    pub fn zones(&self) -> Vec<Interval<'_>> {
        self.main().zones()
    }

    /// Per-zone average/sum of curve `mnemonic` on the main bore (see
    /// [`Sidetrack::zone_stats`]). Broadcast across a project via
    /// `geo.wells().iter().map(|w| w.zone_stats(..))`.
    pub fn zone_stats(&self, mnemonic: &str) -> Vec<(String, Stats)> {
        self.main().zone_stats(mnemonic)
    }

    /// Push the project lithostratigraphic order into every bore, so `zones()` /
    /// `zone_stats()` (on the well and each sidetrack) present zones in it.
    /// Called by the manager after loading a tops file.
    pub fn set_strat_order(&mut self, order: &[String]) {
        for st in self.sidetracks.values_mut() {
            st.set_strat_order(order);
        }
    }
}

/// A single bore: an ordered set of trajectories with one active. Carries the
/// owning well's `head`/`kb` so it can normalize survey input on insert.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct Sidetrack {
    /// The bore label (`""` for the main bore).
    pub label: String,
    head: (f64, f64),
    kb: f64,
    trajectories: Vec<Trajectory>,
    active: usize,
    logs: Vec<Log>,
    /// Formation tops, kept sorted ascending by MD so the next top resolves a
    /// base. Invariant maintained by [`add_tops`](Sidetrack::add_tops).
    tops: Vec<Top>,
    /// Project-wide lithostratigraphic order (top names, shallow→deep), pushed
    /// down from the manager at tops-load time via [`set_strat_order`]. Empty
    /// until set; when non-empty, [`zones`](Sidetrack::zones) returns zones in
    /// this order instead of plain MD order.
    ///
    /// [`set_strat_order`]: Sidetrack::set_strat_order
    strat_order: Vec<String>,
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
            logs: Vec::new(),
            tops: Vec::new(),
            strat_order: Vec::new(),
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

    /// Add a log to this sidetrack.
    pub fn add_log(&mut self, log: Log) {
        self.logs.push(log);
    }

    /// Add formation tops, keeping the set sorted ascending by MD (so the next
    /// top resolves an interval base).
    pub fn add_tops(&mut self, tops: Vec<Top>) {
        self.tops.extend(tops);
        self.sort_tops();
    }

    /// Set the project-wide lithostratigraphic order (top names, shallow→deep).
    /// Pushed down from the manager once a tops file is loaded; [`zones`] then
    /// presents zones in this order, and a coincident-MD (zero-thickness)
    /// cluster's downward interval is assigned to its stratigraphically *lowest*
    /// member (see [`sort_tops`](Sidetrack::sort_tops)).
    ///
    /// [`zones`]: Sidetrack::zones
    pub fn set_strat_order(&mut self, order: &[String]) {
        self.strat_order = order.to_vec();
        self.sort_tops();
    }

    /// Keep `tops` ordered ascending by MD (so the next top resolves a base).
    /// When a lithostratigraphic column is set, equal-MD ties — zero-thickness
    /// pinch-outs where several picks share a depth — break by **strat rank**, so
    /// the *deepest* member of the cluster sorts last and therefore owns the
    /// interval down to the next distinct-MD pick (`zones`/`top` compute base =
    /// the next top's MD). Without a column, or for names absent from it, ties
    /// keep insertion order (stable sort). Geometry for distinct MDs is
    /// unaffected — rank only ever orders within an exact MD tie.
    fn sort_tops(&mut self) {
        if self.strat_order.is_empty() {
            self.tops.sort_by(|a, b| a.md.total_cmp(&b.md));
            return;
        }
        let rank: HashMap<&str, usize> = self
            .strat_order
            .iter()
            .enumerate()
            .map(|(i, n)| (n.as_str(), i))
            .collect();
        let rank_of = |t: &Top| rank.get(t.name.as_str()).copied().unwrap_or(usize::MAX);
        self.tops
            .sort_by(|a, b| a.md.total_cmp(&b.md).then(rank_of(a).cmp(&rank_of(b))));
    }

    /// The interval named by top `name` (case-insensitive): `[top.md, base)`,
    /// where `base` is the next top's MD by sorted MD, or — for the deepest top
    /// — total depth (the active trajectory's `md_range().1`). `None` if no top
    /// matches.
    pub fn top(&self, name: &str) -> Option<Interval<'_>> {
        let i = self
            .tops
            .iter()
            .position(|t| t.name.eq_ignore_ascii_case(name))?;
        let top = &self.tops[i];
        let base = self
            .tops
            .get(i + 1)
            .map(|n| n.md)
            .or_else(|| self.trajectories.get(self.active).map(|t| t.md_range().1))
            .unwrap_or(f64::NAN);
        Some(Interval::new(top.name.clone(), top.md, base, &self.logs))
    }

    /// A full-curve view of log `mnemonic` (case-insensitive), or `None`.
    pub fn log(&self, mnemonic: &str) -> Option<LogView<'_>> {
        self.logs
            .iter()
            .find(|l| l.mnemonic.eq_ignore_ascii_case(mnemonic))
            .map(|l| l.view())
    }

    /// All logs on this bore, in insertion order. Lets a consumer enumerate
    /// every curve (e.g. to assemble model-ready well curves) rather than only
    /// fetch one by mnemonic.
    pub fn logs(&self) -> impl Iterator<Item = &Log> {
        self.logs.iter()
    }

    /// Every formation zone as an [`Interval`] `[top.md, base)` — each top's
    /// base is the next top's MD, or total depth for the deepest.
    ///
    /// Returned in the project **lithostratigraphic order** when one has been
    /// set ([`set_strat_order`], from a loaded tops file), else in MD order. The
    /// reorder is stable and the column is MD-consistent for every separated
    /// pair, so it only ever permutes equal-MD (zero-thickness) groups — each
    /// zone's geometry `[top_md, base)` is unchanged. The basis for per-zone
    /// aggregation.
    ///
    /// [`set_strat_order`]: Sidetrack::set_strat_order
    pub fn zones(&self) -> Vec<Interval<'_>> {
        let td = self.trajectories.get(self.active).map(|t| t.md_range().1);
        let mut zones: Vec<Interval<'_>> = self
            .tops
            .iter()
            .enumerate()
            .map(|(i, top)| {
                let base = self
                    .tops
                    .get(i + 1)
                    .map(|n| n.md)
                    .or(td)
                    .unwrap_or(f64::NAN);
                Interval::new(top.name.clone(), top.md, base, &self.logs)
            })
            .collect();
        if !self.strat_order.is_empty() {
            let rank: HashMap<&str, usize> = self
                .strat_order
                .iter()
                .enumerate()
                .map(|(i, n)| (n.as_str(), i))
                .collect();
            // Stable: names absent from the column (rank MAX) keep MD order.
            zones.sort_by_key(|z| rank.get(z.name.as_str()).copied().unwrap_or(usize::MAX));
        }
        zones
    }

    /// Per-zone statistics of curve `mnemonic` (case-insensitive): one
    /// `(zone_name, Stats)` per zone the curve is defined in. `Stats` carries
    /// the **average** (`mean`) and **sum** (and percentiles); zones where the
    /// curve has no samples are omitted.
    pub fn zone_stats(&self, mnemonic: &str) -> Vec<(String, Stats)> {
        self.zones()
            .into_iter()
            .filter_map(|z| z.log(mnemonic).map(|lv| (z.name.clone(), lv.stats())))
            .collect()
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

    fn ntg_log() -> Log {
        // NTG sampled every 10 MD from 2400 to 2500.
        Log::new(
            "NTG",
            "v/v",
            vec![
                2400.0, 2410.0, 2420.0, 2430.0, 2440.0, 2450.0, 2460.0, 2470.0, 2480.0, 2490.0,
                2500.0,
            ],
            vec![0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0, 1.0],
        )
        .unwrap()
    }

    #[test]
    fn top_resolves_interval_to_next_top() {
        let mut w = Well::new("w", (0.0, 0.0), 0.0);
        let st = w.sidetrack_mut("");
        st.add_trajectory(vertical(0.0, 3000.0)).unwrap();
        st.add_tops(vec![Top::new("Brent", 2400.0), Top::new("Dunlin", 2450.0)]);
        st.add_log(ntg_log());

        let brent = w.top("Brent").unwrap();
        assert_eq!(brent.top_md, 2400.0);
        assert_eq!(brent.base_md, 2450.0); // next top
        assert_eq!(brent.thickness_md(), 50.0);

        // NTG clipped to [2400, 2450): samples 2400..2440 → 0.1..0.5
        let v = brent.log("NTG").unwrap();
        assert_eq!(v.md(), &[2400.0, 2410.0, 2420.0, 2430.0, 2440.0]);
        let s = v.stats();
        assert_eq!(s.count, 5);
        assert_relative_eq!(s.mean, 0.3, epsilon = 1e-12);
    }

    #[test]
    fn zone_stats_average_and_sum_per_zone() {
        let mut w = Well::new("w", (0.0, 0.0), 0.0);
        let st = w.sidetrack_mut("");
        st.add_trajectory(vertical(0.0, 2500.0)).unwrap(); // TD 2500
        st.add_tops(vec![Top::new("Brent", 2400.0), Top::new("Dunlin", 2450.0)]);
        st.add_log(ntg_log()); // NTG 0.1..1.0 at MD 2400..2500 step 10

        let zs = w.zone_stats("NTG");
        assert_eq!(zs.len(), 2);
        // Brent [2400,2450): 0.1..0.5 → mean 0.3, sum 1.5.
        assert_eq!(zs[0].0, "Brent");
        assert_relative_eq!(zs[0].1.mean, 0.3, epsilon = 1e-12);
        assert_relative_eq!(zs[0].1.sum, 1.5, epsilon = 1e-12);
        // Dunlin [2450,2500): 0.6..1.0 → mean 0.8, sum 4.0.
        assert_eq!(zs[1].0, "Dunlin");
        assert_relative_eq!(zs[1].1.mean, 0.8, epsilon = 1e-12);
        assert_relative_eq!(zs[1].1.sum, 4.0, epsilon = 1e-12);
        // zones() exposes both intervals in MD order.
        let zones = w.zones();
        assert_eq!(zones.len(), 2);
        assert_eq!(zones[0].name, "Brent");
    }

    #[test]
    fn strat_order_assigns_coincident_interval_to_deepest_member() {
        let mut w = Well::new("w", (0.0, 0.0), 0.0);
        let st = w.sidetrack_mut("");
        st.add_trajectory(vertical(0.0, 2500.0)).unwrap(); // TD 2500
                                                           // "B" and "Sand" are coincident at 2420 (zero thickness); Sand is
                                                           // added last. The interval below them runs to TD (2500).
        st.add_tops(vec![
            Top::new("A", 2400.0),
            Top::new("B", 2420.0),
            Top::new("Sand", 2420.0),
        ]);

        let base = |w: &Well, name: &str| {
            w.zones()
                .into_iter()
                .find(|z| z.name == name)
                .map(|z| (z.top_md, z.base_md))
        };

        // No column → MD order; the tie falls to insertion order, so Sand
        // (added last) sorts last in the cluster and owns the interval to TD.
        let md_order: Vec<_> = w.zones().iter().map(|z| z.name.clone()).collect();
        assert_eq!(md_order, ["A", "B", "Sand"]);
        assert_eq!(base(&w, "Sand"), Some((2420.0, 2500.0)));
        assert_eq!(base(&w, "B"), Some((2420.0, 2420.0))); // zero thickness

        // A column placing B *below* Sand makes B the deepest of the {B, Sand}
        // cluster, so B now owns the interval to TD; Sand pinches to zero. The
        // sequence is also presented in strat order.
        w.set_strat_order(&["A".to_string(), "Sand".to_string(), "B".to_string()]);
        let strat: Vec<_> = w.zones().iter().map(|z| z.name.clone()).collect();
        assert_eq!(strat, ["A", "Sand", "B"]);
        assert_eq!(base(&w, "B"), Some((2420.0, 2500.0))); // deepest owns the interval
        assert_eq!(base(&w, "Sand"), Some((2420.0, 2420.0))); // shallower → zero
        assert_eq!(base(&w, "A"), Some((2400.0, 2420.0))); // distinct MD: unaffected

        // A name absent from the column keeps insertion order within the tie.
        w.set_strat_order(&["Sand".to_string(), "A".to_string()]);
        let mixed: Vec<_> = w.zones().iter().map(|z| z.name.clone()).collect();
        assert_eq!(mixed, ["Sand", "A", "B"]);
    }

    #[test]
    fn deepest_top_runs_to_td() {
        let mut w = Well::new("w", (0.0, 0.0), 0.0);
        let st = w.sidetrack_mut("");
        st.add_trajectory(vertical(0.0, 2500.0)).unwrap(); // TD = 2500
        st.add_tops(vec![Top::new("Brent", 2400.0), Top::new("Dunlin", 2450.0)]);
        let dunlin = w.top("Dunlin").unwrap();
        assert_eq!(dunlin.top_md, 2450.0);
        assert_eq!(dunlin.base_md, 2500.0); // last top → TD
    }

    #[test]
    fn enumerate_logs_and_mnemonics() {
        let mut w = Well::new("w", (0.0, 0.0), 0.0);
        let st = w.sidetrack_mut("");
        st.add_log(ntg_log());
        st.add_log(Log::new("GR", "GAPI", vec![2400.0, 2410.0], vec![40.0, 60.0]).unwrap());
        assert_eq!(w.logs().count(), 2);
        assert_eq!(w.mnemonics(), vec!["NTG", "GR"]); // insertion order
    }

    #[test]
    fn ergonomic_chain_stats() {
        // well.top("Brent")?.log("NTG")?.stats()
        let mut w = Well::new("15/9-A1", (0.0, 0.0), 0.0);
        let st = w.sidetrack_mut("");
        st.add_trajectory(vertical(0.0, 3000.0)).unwrap();
        st.add_tops(vec![Top::new("Brent", 2400.0), Top::new("Dunlin", 2450.0)]);
        st.add_log(ntg_log());
        let stats = w.top("Brent").unwrap().log("NTG").unwrap().stats();
        assert_relative_eq!(stats.mean, 0.3, epsilon = 1e-12);
        // Case-insensitive top + log lookup.
        assert!(w.top("brent").unwrap().log("ntg").is_some());
        assert!(w.top("Nope").is_none());
    }

    #[test]
    fn well_log_returns_full_curve() {
        let mut w = Well::new("w", (0.0, 0.0), 0.0);
        let st = w.sidetrack_mut("");
        st.add_log(ntg_log());
        let v = w.log("NTG").unwrap();
        assert_eq!(v.md().len(), 11);
        assert!(w.log("GR").is_none());
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
