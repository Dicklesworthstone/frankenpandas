//! Series.replace on Float64 — 2M, small replacement set (low-card values). Isolated -cc bench.
//! Run: cargo run -p fp-frame --example bench_replace_cc --release -- 2000000 30

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
    let labels: Vec<IndexLabel> = (0..n as i64).map(IndexLabel::Int64).collect();
    // values in 0..50 (low card), replace a few of them
    let vals: Vec<Scalar> = (0..n).map(|i| Scalar::Float64((i % 50) as f64)).collect();
    let s = Series::from_values("v", labels, vals).unwrap();
    let repl: Vec<(Scalar, Scalar)> = vec![
        (Scalar::Float64(1.0), Scalar::Float64(101.0)),
        (Scalar::Float64(2.0), Scalar::Float64(102.0)),
        (Scalar::Float64(3.0), Scalar::Float64(103.0)),
    ];

    let mut best = u128::MAX;
    for _ in 0..iters {
        let t = Instant::now();
        let out = s.replace(&repl).expect("replace");
        let e = t.elapsed().as_nanos();
        std::hint::black_box(&out);
        if e < best {
            best = e;
        }
    }
    println!("replace_f64 n={n}: best={best}ns");
}
