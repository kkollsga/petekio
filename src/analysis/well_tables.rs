//! `well_tables` — well-derived tabular + bundle assembly kernels.
//!
//! The numeric orchestration behind the Python bindings' `zone_table`,
//! `net_zone_stats`, and `well.view()` lives here — **one home per formula** —
//! so the binding stays a thin marshaller (validate args → call in → build the
//! pandas frame / bundle dict). Each function takes core types (`Sidetrack`,
//! `Interval`) + primitives and returns plain Rust data (no pandas / PyO3
//! coupling), so it is QC-able in isolation and runs off the GIL.
//!
//! - [`build_zone_table`] — per-`zone × bore` pooling/aggregation for a curve.
//! - [`net_zone_samples`] — net-conditioned per-zone sample selection.
//! - [`gather_raw_logs`] — the master-MD-grid resample feeding the log viewer.
//!
//! Imports from `foundation`, `algorithms`, `core`, and sibling `analysis`
//! kernels (`interpret::net_flags`, `normalize::canonical_mnemonic`).

use crate::algorithms::wells::dz_weights;
use crate::analysis::interpret::{net_flags, Cutoffs};
use crate::analysis::normalize::canonical_mnemonic;
use crate::core::log::LogKind;
use crate::core::well::Sidetrack;
use crate::foundation::Stats;

// -------------------------------------------------------------------------
// zone_table
// -------------------------------------------------------------------------

/// One stat's value on a resolved `Stats` (or the special counts). `gross` is
/// **not** a sample stat — it is handled by the caller (zone geometry), so it is
/// absent here.
fn stat_value(s: &Stats, name: &str) -> f64 {
    match name {
        "mean" => s.mean,
        "sum" => s.sum,
        "count" | "samples" => s.count as f64,
        "min" => s.min,
        "max" => s.max,
        "std" => s.std,
        "p10" => s.p10,
        "p50" => s.p50,
        "p90" => s.p90,
        _ => f64::NAN,
    }
}

/// The crunched result of [`build_zone_table`], ready for the binding to wrap in
/// a `pandas.DataFrame`. Columns are one `Vec<f64>` per requested stat, aligned
/// row-for-row with `zone`/`bore`.
pub enum ZoneTable {
    /// Flat tidy rows (bore-outer), plus the ordered zone categories (for a
    /// `pandas` ordered Categorical that survives `pivot`/`groupby`).
    Tidy {
        zone: Vec<String>,
        bore: Vec<String>,
        cols: Vec<Vec<f64>>,
        categories: Vec<String>,
    },
    /// Grouped rows: per zone a pooled `"all"` row first (sample-weighted across
    /// bores on re-pooled raw samples), then the per-bore rows.
    Aggregate {
        zone: Vec<String>,
        bore: Vec<String>,
        cols: Vec<Vec<f64>>,
    },
}

/// Optional net-conditioning for [`build_zone_table`]: keep only **net** samples
/// (those passing the φ/Sw[/Vsh] cutoffs, with the conditioning curves resampled
/// onto the value curve's MDs) before pooling each zone. The value behind the
/// Python `zone_table(cut=NetSettings(...))` — mirroring
/// [`net_zone_samples`]'s conditioning walk.
pub struct NetCond<'a> {
    pub cut: Cutoffs,
    pub phi: &'a str,
    pub sw: &'a str,
    pub vsh: Option<&'a str>,
}

/// Build a per-`zone × bore` table of `curve` over `bores`, where each entry is a
/// display label with its sidetrack. `stats` are already-validated names (`Stats`
/// attributes plus `samples`/`gross`). Bores without a trajectory are skipped;
/// zones come from each bore's `zones()` (lithostratigraphic order). `count == 0`
/// cells are dropped unless `include_empty`. `weighted` thickness-weights every
/// average by each sample's MD span (via [`dz_weights`]); `false` uses the plain
/// sample mean. `zones` (lowercased) keeps only those zone names. When `net` is
/// `Some`, each cell is **net-conditioned** first (only net samples pooled).
/// Exactly one of aggregate or tidy is produced — the caller enforces the
/// pivot/aggregate mutual exclusion. Without `net`, bit-identical to the former
/// in-binding implementation.
#[allow(clippy::too_many_arguments)]
pub fn build_zone_table(
    bores: &[(String, &Sidetrack)],
    curve: &str,
    stats: &[&str],
    zones: Option<&[String]>,
    include_empty: bool,
    aggregate: bool,
    weighted: bool,
    net: Option<NetCond<'_>>,
) -> ZoneTable {
    let zone_stats = |vals: &[f64], w: &[f64]| {
        if weighted {
            Stats::weighted(vals, w)
        } else {
            Stats::of(vals)
        }
    };
    // Optional zone filter: keep only these names (case-insensitive, exact).
    let keep: Option<std::collections::HashSet<String>> =
        zones.map(|z| z.iter().map(|s| s.to_ascii_lowercase()).collect());

    // One pass: per-bore rows (bore-outer, non-empty unless include_empty), the
    // zone first-appearance order, and the pooled (value, weight) pairs per zone.
    let mut order: Vec<String> = Vec::new();
    let mut rows: Vec<(String, String, Vec<f64>)> = Vec::new(); // (zone, bore, stat values)
    let mut pooled: std::collections::HashMap<String, (Vec<f64>, Vec<f64>)> =
        std::collections::HashMap::new();
    for (label, st) in bores {
        if st.trajectories().is_empty() {
            continue; // no md_range — nothing positioned
        }
        // Whole-bore conditioning curves (resampled onto each interval's MDs
        // below) when net-conditioning is requested.
        let (phi_c, sw_c, vsh_c) = match &net {
            Some(nc) => (
                st.log(nc.phi),
                st.log(nc.sw),
                nc.vsh.and_then(|m| st.log(m)),
            ),
            None => (None, None, None),
        };
        for iv in st.zones() {
            if let Some(k) = &keep {
                if !k.contains(&iv.name.to_ascii_lowercase()) {
                    continue; // not in the requested zone subset
                }
            }
            if !order.contains(&iv.name) {
                order.push(iv.name.clone());
            }
            let gross = iv.thickness_md();
            let s = iv.log(curve).map(|l| {
                // Net-condition (keep only samples passing the cutoffs) when asked,
                // else take every sample. `dz_weights` recomputes over the kept MDs.
                let (kept_md, kept_vals): (Vec<f64>, Vec<f64>) = if let Some(nc) = &net {
                    let md = l.md();
                    let sample = |c: &Option<crate::LogView<'_>>| -> Vec<f64> {
                        c.as_ref()
                            .map(|x| x.resample_onto(md))
                            .unwrap_or_else(|| vec![f64::NAN; md.len()])
                    };
                    let phi_at = sample(&phi_c);
                    let sw_at = sample(&sw_c);
                    let flags = if vsh_c.is_some() {
                        net_flags(&phi_at, &sw_at, Some(&sample(&vsh_c)), &nc.cut)
                    } else {
                        net_flags(&phi_at, &sw_at, None, &nc.cut)
                    };
                    md.iter()
                        .zip(l.values())
                        .zip(&flags)
                        .filter(|(_, n)| **n)
                        .map(|((m, v), _)| (*m, *v))
                        .unzip()
                } else {
                    (l.md().to_vec(), l.values().to_vec())
                };
                let w = dz_weights(&kept_md);
                let st = zone_stats(&kept_vals, &w);
                let e = pooled.entry(iv.name.clone()).or_default();
                e.0.extend_from_slice(&kept_vals);
                e.1.extend_from_slice(&w);
                st
            });
            let count = s.as_ref().map(|x| x.count).unwrap_or(0);
            if count == 0 && !include_empty {
                continue;
            }
            let vals = stats
                .iter()
                .map(|n| match *n {
                    "gross" => gross,
                    _ => s.as_ref().map(|s| stat_value(s, n)).unwrap_or(f64::NAN),
                })
                .collect();
            rows.push((iv.name.clone(), label.clone(), vals));
        }
    }

    if aggregate {
        let mut zone_col: Vec<String> = Vec::new();
        let mut bore_col: Vec<String> = Vec::new();
        let mut cols: Vec<Vec<f64>> = vec![Vec::new(); stats.len()];
        for zone in &order {
            let (pv, pw) = match pooled.get(zone) {
                Some((v, w)) => (v.as_slice(), w.as_slice()),
                None => (&[][..], &[][..]),
            };
            let ps = zone_stats(pv, pw);
            let zrows: Vec<&(String, String, Vec<f64>)> =
                rows.iter().filter(|(z, _, _)| z == zone).collect();
            if ps.count == 0 && zrows.is_empty() && !include_empty {
                continue;
            }
            zone_col.push(zone.clone());
            bore_col.push("all".to_string());
            for (k, name) in stats.iter().enumerate() {
                // `gross` isn't a sample stat — its pooled value is the mean zone
                // thickness across the bores shown.
                let v = if *name == "gross" {
                    let g: Vec<f64> = zrows.iter().map(|(_, _, vals)| vals[k]).collect();
                    if g.is_empty() {
                        f64::NAN
                    } else {
                        g.iter().sum::<f64>() / g.len() as f64
                    }
                } else {
                    stat_value(&ps, name)
                };
                cols[k].push(v);
            }
            for (_, b, vals) in zrows {
                zone_col.push(zone.clone());
                bore_col.push(b.clone());
                for (k, v) in vals.iter().enumerate() {
                    cols[k].push(*v);
                }
            }
        }
        return ZoneTable::Aggregate {
            zone: zone_col,
            bore: bore_col,
            cols,
        };
    }

    // Flat tidy / pivot.
    let mut zone_col: Vec<String> = Vec::with_capacity(rows.len());
    let mut bore_col: Vec<String> = Vec::with_capacity(rows.len());
    let mut cols: Vec<Vec<f64>> = vec![Vec::new(); stats.len()];
    for (z, b, vals) in &rows {
        zone_col.push(z.clone());
        bore_col.push(b.clone());
        for (k, v) in vals.iter().enumerate() {
            cols[k].push(*v);
        }
    }
    let present: std::collections::HashSet<&str> = zone_col.iter().map(String::as_str).collect();
    let categories: Vec<String> = order
        .into_iter()
        .filter(|z| present.contains(z.as_str()))
        .collect();
    ZoneTable::Tidy {
        zone: zone_col,
        bore: bore_col,
        cols,
        categories,
    }
}

// -------------------------------------------------------------------------
// net_zone_stats
// -------------------------------------------------------------------------

/// Net-conditioned per-zone sample selection for curve `value` on `st`: for each
/// zone, keep only the **net** samples (those passing the φ/Sw[/Vsh] cutoffs,
/// with the conditioning curves resampled onto `value`'s MDs). Returns
/// `[(zone_name, kept_values)]` in lithostratigraphic order; the caller turns
/// each kept-sample vector into arithmetic or geometric `Stats`. Zones with no
/// net samples yield an empty vector. Bit-identical to the former in-binding
/// conditioning walk (now an O(n+k) [`resample_onto`](crate::LogView::resample_onto)).
pub fn net_zone_samples(
    st: &Sidetrack,
    value: &str,
    phi: &str,
    sw: &str,
    vsh: Option<&str>,
    cut: &Cutoffs,
) -> Vec<(String, Vec<f64>)> {
    // Whole-bore curves used to condition each zone (resampled onto its MDs).
    let phi_v = st.log(phi);
    let sw_v = st.log(sw);
    let vsh_v = vsh.and_then(|m| st.log(m));
    let sample = |v: &Option<crate::LogView<'_>>, md: &[f64]| -> Vec<f64> {
        v.as_ref()
            .map(|c| c.resample_onto(md))
            .unwrap_or_else(|| vec![f64::NAN; md.len()])
    };

    let mut out: Vec<(String, Vec<f64>)> = Vec::new();
    for iv in st.zones() {
        let name = iv.name.clone();
        let (md, vals): (Vec<f64>, Vec<f64>) = match iv.log(value) {
            Some(lv) => (lv.md().to_vec(), lv.values().to_vec()),
            None => (Vec::new(), Vec::new()),
        };
        let phi_at = sample(&phi_v, &md);
        let sw_at = sample(&sw_v, &md);
        let net = if vsh_v.is_some() {
            let vsh_at = sample(&vsh_v, &md);
            net_flags(&phi_at, &sw_at, Some(&vsh_at), cut)
        } else {
            net_flags(&phi_at, &sw_at, None, cut)
        };
        let kept: Vec<f64> = vals
            .iter()
            .zip(&net)
            .filter(|(_, n)| **n)
            .map(|(v, _)| *v)
            .collect();
        out.push((name, kept));
    }
    out
}

// -------------------------------------------------------------------------
// well.view() raw-log gather
// -------------------------------------------------------------------------

/// One curve resampled onto a well's master MD grid, canonicalized for the viewer.
pub struct RawCurve {
    pub mnemonic: String,
    pub canonical: String,
    pub unit: String,
    pub core: bool,
    pub values: Vec<f64>,
}

/// A formation zone with both MD and TVD depths for the viewer.
pub struct RawZone {
    pub name: String,
    pub top_md: f64,
    pub base_md: f64,
    pub top_tvd: f64,
    pub base_tvd: f64,
}

/// A bore's raw log data on a shared MD grid, ready for the viewer bundle.
pub struct RawWellLogs {
    pub md: Vec<f64>,
    pub tvd: Vec<f64>,
    pub curves: Vec<RawCurve>,
    pub zones: Vec<RawZone>,
}

/// Whether curve `mnemonic` is wanted given an optional filter (matched on the
/// raw mnemonic or its canonical form, case-insensitive; `None` keeps all).
fn wanted(mnemonic: &str, filter: Option<&[String]>) -> bool {
    match filter {
        None => true,
        Some(list) => {
            let canon = canonical_mnemonic(mnemonic);
            list.iter()
                .any(|q| q.eq_ignore_ascii_case(mnemonic) || q.eq_ignore_ascii_case(&canon))
        }
    }
}

/// Gather bore `st`'s selected logs onto one master MD grid (the sorted-unique
/// union of the selected curves' depths). Each curve is resampled onto that grid
/// (linear in-span, `NaN` outside); `tvd` is trajectory TVDSS where a trajectory
/// exists, else the vertical assumption `md - kb`. Bit-identical to the former
/// in-binding gather. `kb` is the owning well's datum. The caller adds the
/// well-level fields (`id`/`x`/`y`/`datum_m`) around this.
pub fn gather_raw_logs(st: &Sidetrack, kb: f64, filter: Option<&[String]>) -> RawWellLogs {
    let logs: Vec<_> = st.logs().filter(|l| wanted(&l.mnemonic, filter)).collect();

    // Master MD grid: sorted-unique union of the selected curves' depths.
    let mut md: Vec<f64> = Vec::new();
    for l in &logs {
        md.extend_from_slice(l.view().md());
    }
    md.sort_by(|a, b| a.total_cmp(b));
    md.dedup();

    // TVDSS at each MD: trajectory when present, else the vertical assumption.
    let tvd: Vec<f64> = md.iter().map(|&d| st.tvd(d).unwrap_or(d - kb)).collect();

    let curves: Vec<RawCurve> = logs
        .iter()
        .map(|l| {
            let view = l.view();
            let values: Vec<f64> = md
                .iter()
                .map(|&d| view.at_md(d).unwrap_or(f64::NAN))
                .collect();
            RawCurve {
                mnemonic: l.mnemonic.clone(),
                canonical: canonical_mnemonic(&l.mnemonic),
                unit: l.unit.clone(),
                core: matches!(l.kind(), LogKind::Core),
                values,
            }
        })
        .collect();

    let zones: Vec<RawZone> = st
        .zones()
        .into_iter()
        .map(|iv| RawZone {
            name: iv.name.clone(),
            top_md: iv.top_md,
            base_md: iv.base_md,
            top_tvd: st.tvd(iv.top_md).unwrap_or(iv.top_md - kb),
            base_tvd: st.tvd(iv.base_md).unwrap_or(iv.base_md - kb),
        })
        .collect();

    RawWellLogs {
        md,
        tvd,
        curves,
        zones,
    }
}
