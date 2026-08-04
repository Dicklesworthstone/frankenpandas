use std::collections::BTreeMap;

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::Index;
fn timeit<F: FnMut()>(label: &str, mut f: F) {
    let mut best = u128::MAX;
    for _ in 0..6 {
        let t = std::time::Instant::now();
        f();
        best = best.min(t.elapsed().as_nanos());
    }
    println!("{label}: {:.2}ms", best as f64 / 1e6);
}
fn main() {
    let n: usize = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(5_000_000);
    let groups: i64 = std::env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(1000);
    let idx = Index::from_range(0, n as i64, 1);
    let mut cols = BTreeMap::new();
    cols.insert(
        "k".to_string(),
        Column::from_i64_values((0..n as i64).map(|i| i % groups).collect()),
    );
    cols.insert(
        "v".to_string(),
        Column::from_i64_values(
            (0..n as u64)
                .map(|i| {
                    let mut z = i.wrapping_mul(0x9E3779B97F4A7C15);
                    z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
                    (z >> 20) as i64
                })
                .collect(),
        ),
    );
    let df = DataFrame::new(idx, cols).unwrap();
    timeit("gb.mean i64", || {
        std::hint::black_box(df.groupby(&["k"]).unwrap().mean().unwrap().shape());
    });
    timeit("gb.sum i64", || {
        std::hint::black_box(df.groupby(&["k"]).unwrap().sum().unwrap().shape());
    });
    timeit("gb.std i64", || {
        std::hint::black_box(df.groupby(&["k"]).unwrap().std().unwrap().shape());
    });
    timeit("gb.var i64", || {
        std::hint::black_box(df.groupby(&["k"]).unwrap().var().unwrap().shape());
    });
    timeit("gb.median i64", || {
        std::hint::black_box(df.groupby(&["k"]).unwrap().median().unwrap().shape());
    });
    timeit("gb.min i64", || {
        std::hint::black_box(df.groupby(&["k"]).unwrap().min().unwrap().shape());
    });
}
