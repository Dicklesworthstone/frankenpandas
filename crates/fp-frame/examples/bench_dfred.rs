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
        .unwrap_or(2_000_000);
    let ncols = 8;
    let idx = Index::from_range(0, n as i64, 1);
    let mut cols = BTreeMap::new();
    for c in 0..ncols {
        let name = format!("c{c}");
        cols.insert(
            name,
            Column::from_i64_values(
                (0..n as u64)
                    .map(|i| {
                        let mut z = i.wrapping_mul(0x9E3779B97F4A7C15 + c as u64);
                        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
                        (z >> 13) as i64
                    })
                    .collect(),
            ),
        );
    }
    let df = DataFrame::new(idx, cols).unwrap();
    timeit("df.sum i64", || {
        std::hint::black_box(df.sum().unwrap().len());
    });
    timeit("df.mean i64", || {
        std::hint::black_box(df.mean().unwrap().len());
    });
    timeit("df.max i64", || {
        std::hint::black_box(df.max().unwrap().len());
    });
    timeit("df.std i64", || {
        std::hint::black_box(df.std().unwrap().len());
    });
    timeit("df.median i64", || {
        std::hint::black_box(df.median().unwrap().len());
    });
}
