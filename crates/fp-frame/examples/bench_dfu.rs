//! DataFrame sort_values / drop_duplicates by Scalar-backed Utf8. bench_dfu <n> <card>
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
        .unwrap_or(10000);
    let cats: Vec<String> = (0..card).map(|c| format!("value_{c:06}")).collect();
    let mut cols = BTreeMap::new();
    cols.insert(
        "k".to_string(),
        Column::from_values(
            (0..n)
                .map(|i| Scalar::Utf8(cats[(sm(i, 0) as usize) % card].clone()))
                .collect(),
        )
        .unwrap(),
    );
    cols.insert(
        "a".to_string(),
        Column::from_f64_values((0..n).map(|i| (sm(i, 7) % 100000) as f64).collect()),
    );
    let labels: Vec<IndexLabel> = (0..n as i64).map(IndexLabel::Int64).collect();
    let df =
        DataFrame::new_with_column_order(Index::new(labels), cols, vec!["k".into(), "a".into()])
            .unwrap();
    let subset = vec!["k".to_string()];
    timeit("sort_values_utf8", || {
        std::hint::black_box(df.sort_values("k", true).unwrap().shape());
    });
    timeit("set_index_utf8", || {
        std::hint::black_box(df.set_index("k", true).unwrap().shape());
    });
    timeit("drop_dup_utf8", || {
        std::hint::black_box(
            df.drop_duplicates(Some(&subset), fp_index::DuplicateKeep::First, false)
                .unwrap()
                .shape(),
        );
    });
    timeit("duplicated_utf8", || {
        std::hint::black_box(
            df.duplicated(Some(&subset), fp_index::DuplicateKeep::First)
                .unwrap()
                .len(),
        );
    });
}
