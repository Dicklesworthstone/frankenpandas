//! Series.loc[[labels]] on a unique all-Utf8 index. Exercises the identity-cached
//! String->position hashtable (sfsx4); the non-Int64 general path otherwise rebuilds
//! a per-call pointer-key FxHashMap<&IndexLabel, Vec<usize>> over the whole index.
//! Run: cargo run -p fp-frame --example bench_loc_utf8_cc --release -- 2000000 1000 50

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

    let labels: Vec<IndexLabel> = (0..n)
        .map(|i| IndexLabel::Utf8(format!("k{i:08}")))
        .collect();
    let s = Series::from_values(
        "v",
        labels,
        (0..n).map(|i| Scalar::Float64(i as f64)).collect(),
    )
    .unwrap();
    let sel: Vec<IndexLabel> = (0..k)
        .map(|j| IndexLabel::Utf8(format!("k{:08}", j * (n / k))))
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
        "loc_utf8 n={n} k={k}: best={best}ns ({:.4}ms)",
        best as f64 / 1e6
    );
}
