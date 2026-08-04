//! Series.map(dict) on Float64 — 2M, 50-entry full-coverage mapping (categorical re-encode).
//! Run: cargo run -p fp-frame --example bench_map_cc --release -- 2000000 30
//! Materialized read: cargo run -p fp-frame --example bench_map_cc --release -- 2000000 30 materialize
//! Numeric materialized read: cargo run -p fp-frame --example bench_map_cc --release -- 2000000 30 numpy

use std::time::Instant;

use fp_frame::Series;
use fp_index::IndexLabel;
use fp_types::Scalar;

#[derive(Clone, Copy)]
enum Mode {
    Deferred,
    Materialize,
    Numpy,
}

impl Mode {
    fn from_arg(mode: Option<&String>) -> Self {
        match mode.map(String::as_str) {
            Some("materialize") => Self::Materialize,
            Some("numpy") => Self::Numpy,
            _ => Self::Deferred,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Deferred => "deferred",
            Self::Materialize => "materialize",
            Self::Numpy => "numpy",
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let n: usize = args
        .get(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(2_000_000);
    let iters: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(30);
    let mode = Mode::from_arg(args.get(3));
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
        let e = match mode {
            Mode::Deferred => {
                let e = t.elapsed().as_nanos();
                std::hint::black_box(&out);
                e
            }
            Mode::Materialize => {
                std::hint::black_box(out.values());
                t.elapsed().as_nanos()
            }
            Mode::Numpy => {
                std::hint::black_box(out.to_numpy());
                t.elapsed().as_nanos()
            }
        };
        if e < best {
            best = e;
        }
    }
    println!("map_f64 mode={} n={n}: best={best}ns", mode.as_str());
}
