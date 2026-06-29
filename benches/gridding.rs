//! Criterion bench for minimum-curvature gridding: cold solve vs warm-started
//! incremental re-grid. The cold path is unchanged from 0.2.0 (warm-start added
//! only an `Option<seed>` branch that is `None` here), so this also guards the
//! cold path against regression. Heavy scratch input → `dev-docs/bench/out/`.

use criterion::{criterion_group, criterion_main, Criterion};
use petekio::{GridGeometry, GridMethod, PointSet};
use std::io::Write;
use std::path::PathBuf;

/// Write a deterministic scattered-point field to a scratch xyz and return it.
fn scatter_path() -> PathBuf {
    let dir = PathBuf::from("dev-docs/bench/out");
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("scatter_bench.xyz");
    let mut s = String::new();
    // 64 points on a coarse lattice with a smooth analytic z + a ripple.
    for i in 0..8u32 {
        for j in 0..8u32 {
            let x = i as f64 * 5.0;
            let y = j as f64 * 5.0;
            let z = 1000.0 + 0.5 * x + 0.3 * y + 5.0 * ((x * 0.2).sin() + (y * 0.2).cos());
            s.push_str(&format!("{x} {y} {z}\n"));
        }
    }
    let mut f = std::fs::File::create(&path).expect("write scratch xyz");
    f.write_all(s.as_bytes()).expect("write scratch xyz");
    path
}

fn geom_40() -> GridGeometry {
    GridGeometry {
        xori: 0.0,
        yori: 0.0,
        xinc: 1.0,
        yinc: 1.0,
        ncol: 40,
        nrow: 40,
        rotation_deg: 0.0,
        yflip: false,
    }
}

fn bench(c: &mut Criterion) {
    let pts = PointSet::load_irap_points(scatter_path()).unwrap();
    let geom = geom_40();
    let cold = pts
        .to_surface(geom.clone(), GridMethod::MinimumCurvature)
        .unwrap();

    c.bench_function("min_curvature_cold_40x40", |b| {
        b.iter(|| {
            pts.to_surface(geom.clone(), GridMethod::MinimumCurvature)
                .unwrap()
        })
    });
    c.bench_function("min_curvature_warm_40x40", |b| {
        b.iter(|| pts.regrid_min_curvature(&cold).unwrap())
    });
}

criterion_group!(benches, bench);
criterion_main!(benches);
