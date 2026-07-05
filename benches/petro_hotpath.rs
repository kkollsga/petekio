//! Criterion bench for the petrophysics hot path — `LogView::at_md`,
//! `LogView::resample`, and the per-sample-conditioning walk that the Python
//! `net_zone_stats` binding runs (three curves sampled onto a value curve's
//! MDs). Realistic field scale: 40 curves × ~4000 samples. Pure in-memory (no
//! IO). This isolates the depth-lookup cost that dominates zone aggregation, and
//! is the regression guard for the linear-scan → binary-search / merge-walk fix.

use criterion::{criterion_group, criterion_main, Criterion};
use petekio::Log;
use std::hint::black_box;

const N_CURVES: usize = 40;
const N_SAMPLES: usize = 4000;

/// A synthetic curve: `N_SAMPLES` ascending MDs from 1000 m at 0.15 m spacing,
/// values a smooth ramp with every 37th sample NaN (undefined).
fn curve(seed: usize) -> Log {
    let lo = 1000.0 + seed as f64 * 0.05;
    let md: Vec<f64> = (0..N_SAMPLES).map(|i| lo + i as f64 * 0.15).collect();
    let values: Vec<f64> = (0..N_SAMPLES)
        .map(|i| {
            if i % 37 == 0 {
                f64::NAN
            } else {
                0.1 + ((i + seed) as f64 * 0.001).sin() * 0.2
            }
        })
        .collect();
    Log::new(format!("C{seed}"), "v/v", md, values).unwrap()
}

fn field() -> Vec<Log> {
    (0..N_CURVES).map(curve).collect()
}

/// MD query points spanning a curve's range (not aligned to its nodes, so every
/// lookup interpolates a real bracket).
fn queries() -> Vec<f64> {
    (0..N_SAMPLES)
        .map(|i| 1000.0 + i as f64 * 0.15 + 0.07)
        .collect()
}

fn bench(c: &mut Criterion) {
    let logs = field();
    let q = queries();

    // Single-curve at_md over a full sweep of query depths.
    c.bench_function("at_md_4000q_1curve", |b| {
        let v = logs[0].view();
        b.iter(|| {
            let mut acc = 0.0;
            for &d in &q {
                acc += v.at_md(black_box(d)).unwrap_or(0.0);
            }
            acc
        })
    });

    // resample every curve onto a regular grid.
    c.bench_function("resample_40curves_step0p2", |b| {
        b.iter(|| {
            let mut n = 0usize;
            for l in &logs {
                n += l.view().resample(black_box(0.2)).len();
            }
            n
        })
    });

    // net_zone_stats analog (OLD path): for each of 40 value curves, sample 3
    // conditioning curves at each of the value curve's MDs (per-sample at_md, ×3
    // curves) — O(k·n) per curve.
    c.bench_function("net_condition_40curves_x3", |b| {
        let phi = &logs[1];
        let sw = &logs[2];
        let vsh = &logs[3];
        b.iter(|| {
            let mut acc = 0.0;
            for val in &logs {
                let md = val.view();
                for &d in md.md() {
                    acc += phi.view().at_md(d).unwrap_or(0.0);
                    acc += sw.view().at_md(d).unwrap_or(0.0);
                    acc += vsh.view().at_md(d).unwrap_or(0.0);
                }
            }
            acc
        })
    });

    // net_zone_stats analog (NEW path): the same conditioning, but each cutoff
    // curve is resampled onto the value curve's MD grid via the O(n+k) tandem
    // merge-walk (`LogView::resample_onto`) the binding now uses. Directly
    // comparable to `net_condition_40curves_x3` above.
    c.bench_function("net_condition_40curves_resample_onto", |b| {
        let phi = logs[1].view();
        let sw = logs[2].view();
        let vsh = logs[3].view();
        b.iter(|| {
            let mut acc = 0.0;
            for val in &logs {
                let targets = val.view();
                let targets = targets.md();
                let pa = phi.resample_onto(black_box(targets));
                let sa = sw.resample_onto(black_box(targets));
                let va = vsh.resample_onto(black_box(targets));
                for (x, (y, z)) in pa.iter().zip(sa.iter().zip(va.iter())) {
                    acc += x + y + z;
                }
            }
            acc
        })
    });
}

criterion_group!(benches, bench);
criterion_main!(benches);
