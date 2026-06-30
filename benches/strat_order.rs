//! Criterion bench for the cross-well lithostratigraphic merge
//! (`algorithms::wells::merge_strat_order`). Realistic field scale: ~40 wells ×
//! ~15 shared horizons, with periodic zero-thickness pinch-outs to exercise the
//! tie path. Pure in-memory (no IO) — isolates the added cost that
//! `load_well_tops` now incurs.

use criterion::{criterion_group, criterion_main, Criterion};
use petekio::algorithms::wells::merge_strat_order;
use std::hint::black_box;

/// `n_wells` wells, each penetrating the shared `names` column at a per-well
/// depth offset; every 5th marker is coincident with the one above it (a
/// pinch-out) so the merge resolves ties across wells.
fn field(n_wells: usize, names: &[String]) -> Vec<Vec<(f64, &str)>> {
    (0..n_wells)
        .map(|w| {
            let base = 2000.0 + w as f64 * 7.0;
            names
                .iter()
                .enumerate()
                .map(|(i, nm)| {
                    let md = base + i as f64 * 20.0;
                    let md = if i % 5 == 0 && i > 0 { md - 20.0 } else { md };
                    (md, nm.as_str())
                })
                .collect()
        })
        .collect()
}

fn bench(c: &mut Criterion) {
    let names: Vec<String> = (0..15).map(|i| format!("Horizon {i}")).collect();
    let wells = field(40, &names);
    c.bench_function("merge_strat_order_40w_15h", |b| {
        b.iter(|| merge_strat_order(black_box(&wells)))
    });
}

criterion_group!(benches, bench);
criterion_main!(benches);
