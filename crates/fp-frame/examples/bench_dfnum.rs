//! DataFrame numeric ops: rank / corr / nlargest. bench_dfnum <n> <ncols>
use std::collections::BTreeMap;

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::{Index, IndexLabel};
fn timeit<F: FnMut()>(label: &str, mut f: F) {
    let mut best = u128::MAX;
    for _ in 0..6 {
        let t = std::time::Instant::now();
        f();
        best = best.min(t.elapsed().as_nanos());
    }
    println!("{label}: {:.2}ms", best as f64 / 1e6);
}
fn sm(i: usize, s: u64) -> u64 {
    let mut h = (i as u64).wrapping_add(s).wrapping_mul(0x9E3779B97F4A7C15);
    h = (h ^ (h >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    h ^ (h >> 31)
}
fn main() {
    let n: usize = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(1_000_000);
    let ncols: usize = std::env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(10);
    let mut cols = BTreeMap::new();
    let mut order = vec![];
    for c in 0..ncols {
        let name = format!("c{c}");
        cols.insert(
            name.clone(),
            Column::from_f64_values(
                (0..n)
                    .map(|i| (sm(i, c as u64) >> 11) as f64 / 1e6)
                    .collect(),
            ),
        );
        order.push(name);
    }
    let df = DataFrame::new_with_column_order(
        Index::new((0..n as i64).map(IndexLabel::Int64).collect()),
        cols,
        order,
    )
    .unwrap();
    timeit("rank_avg", || {
        std::hint::black_box(df.rank("average", true, "keep").unwrap().shape());
    });
    timeit("corr", || {
        std::hint::black_box(df.corr().unwrap().shape());
    });
    timeit("nlargest", || {
        std::hint::black_box(df.nlargest(100, "c0").unwrap().shape());
    });
    timeit("nunique", || {
        std::hint::black_box(df.nunique().unwrap().len());
    });
}
