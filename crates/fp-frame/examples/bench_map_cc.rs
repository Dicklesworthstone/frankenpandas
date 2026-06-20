//! Series.map(dict) on Float64 — 2M, 50-entry full-coverage mapping (categorical re-encode).
//! Run: cargo run -p fp-frame --example bench_map_cc --release -- 2000000 30
//! Materialized read: cargo run -p fp-frame --example bench_map_cc --release -- 2000000 30 materialize

use std::time::Instant;

use fp_frame::Series;
use fp_index::IndexLabel;
use fp_types::Scalar;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let n: usize = args
        .get(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(2_000_000);
    let iters: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(30);
    let materialize = args.get(3).is_some_and(|mode| mode == "materialize");
    let labels: Vec<IndexLabel> = (0..n as i64).map(IndexLabel::Int64).collect();
    let vals: Vec<Scalar> = (0..n).map(|i| Scalar::Float64((i % 50) as f64)).collect();
    let s = Series::from_values("v", labels, vals).unwrap();
    // full-coverage 50-entry mapping
    let mapping: Vec<(Scalar, Scalar)> = (0..50)
        .map(|k| {
            (
                Scalar::Float64(k as f64),
                Scalar::Float64(k as f64 * 10.0 + 1.0),
            )
        })
        .collect();

    let mut best = u128::MAX;
    for _ in 0..iters {
        let t = Instant::now();
        let out = s.map(&mapping).expect("map");
        let e = if materialize {
            std::hint::black_box(out.values());
            t.elapsed().as_nanos()
        } else {
            let e = t.elapsed().as_nanos();
            std::hint::black_box(&out);
            e
        };
        if e < best {
            best = e;
        }
    }
    let mode = if materialize {
        "materialize"
    } else {
        "deferred"
    };
    println!("map_f64 mode={mode} n={n}: best={best}ns");
}
