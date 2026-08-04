//! DataFrameGroupBy on TWO Scalar-backed Utf8 keys. bench_dfgb2u <n> <card>
use std::collections::BTreeMap;

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::{Index, IndexLabel};
use fp_types::Scalar;
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
        .unwrap_or(2_000_000);
    let card: usize = std::env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(100);
    let k1: Vec<String> = (0..card).map(|c| format!("k1_{c:04}")).collect();
    let k2: Vec<String> = (0..card).map(|c| format!("k2_{c:04}")).collect();
    let mut cols = BTreeMap::new();
    cols.insert(
        "k1".to_string(),
        Column::from_values(
            (0..n)
                .map(|i| Scalar::Utf8(k1[(sm(i, 0) as usize) % card].clone()))
                .collect(),
        )
        .unwrap(),
    );
    cols.insert(
        "k2".to_string(),
        Column::from_values(
            (0..n)
                .map(|i| Scalar::Utf8(k2[(sm(i, 1) as usize) % card].clone()))
                .collect(),
        )
        .unwrap(),
    );
    cols.insert(
        "a".to_string(),
        Column::from_f64_values((0..n).map(|i| (sm(i, 7) % 100000) as f64).collect()),
    );
    let labels: Vec<IndexLabel> = (0..n as i64).map(IndexLabel::Int64).collect();
    let df = DataFrame::new_with_column_order(
        Index::new(labels),
        cols,
        vec!["k1".into(), "k2".into(), "a".into()],
    )
    .unwrap();
    timeit("sum", || {
        std::hint::black_box(df.groupby(&["k1", "k2"]).unwrap().sum().unwrap().shape());
    });
    timeit("mean", || {
        std::hint::black_box(df.groupby(&["k1", "k2"]).unwrap().mean().unwrap().shape());
    });
    timeit("count", || {
        std::hint::black_box(df.groupby(&["k1", "k2"]).unwrap().count().unwrap().shape());
    });
    timeit("max", || {
        std::hint::black_box(df.groupby(&["k1", "k2"]).unwrap().max().unwrap().shape());
    });
    timeit("head", || {
        std::hint::black_box(df.groupby(&["k1", "k2"]).unwrap().head(5).unwrap().shape());
    });
    timeit("nth", || {
        std::hint::black_box(df.groupby(&["k1", "k2"]).unwrap().nth(0).unwrap().shape());
    });
}
