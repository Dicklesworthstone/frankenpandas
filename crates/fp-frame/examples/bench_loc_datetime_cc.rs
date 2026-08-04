//! Series.loc[[timestamps]] on a unique Datetime64 (time-series) index.
//! Exercises the identity-cached ns->position hashtable (recbe).
//! Run: cargo run -p fp-frame --example bench_loc_datetime_cc --release -- 2000000 1000 50

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
    let k: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(1000);
    let iters: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(50);
    let base: i64 = 1_600_000_000_000_000_000;
    let step: i64 = 60_000_000_000;
    let labels: Vec<IndexLabel> = (0..n as i64)
        .map(|i| IndexLabel::Datetime64(base + i * step))
        .collect();
    let s = Series::from_values(
        "v",
        labels,
        (0..n).map(|i| Scalar::Float64(i as f64)).collect(),
    )
    .unwrap();
    let sel: Vec<IndexLabel> = (0..k)
        .map(|j| IndexLabel::Datetime64(base + ((j * (n / k)) as i64) * step))
        .collect();

    let mut best = u128::MAX;
    for _ in 0..iters {
        let t = Instant::now();
        let out = s.loc(&sel).expect("loc");
        let e = t.elapsed().as_nanos();
        std::hint::black_box(&out);
        if e < best {
            best = e;
        }
    }
    println!(
        "loc_datetime n={n} k={k}: best={best}ns ({:.4}ms)",
        best as f64 / 1e6
    );
}
