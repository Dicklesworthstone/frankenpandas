//! DataFrame.explode on a delimited Utf8 column. bench_explode <nrows>
use std::collections::BTreeMap;

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::Index;
fn sm(i: usize, s: u64) -> u64 {
    let mut h = (i as u64).wrapping_add(s).wrapping_mul(0x9E3779B97F4A7C15);
    h = (h ^ (h >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    h ^ (h >> 31)
}
fn main() {
    let n: usize = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(500_000);
    let idx = Index::new_known_unique_int64_unit_range(0, n);
    let mut cols = BTreeMap::new();
    // each row: 3 comma-separated tokens
    let mut bytes = Vec::new();
    let mut offs = vec![0usize];
    for i in 0..n {
        let s = format!(
            "t{}-a,t{}-b,t{}-c",
            sm(i, 0) % 1000,
            sm(i, 1) % 1000,
            sm(i, 2) % 1000
        );
        bytes.extend_from_slice(s.as_bytes());
        offs.push(bytes.len());
    }
    cols.insert("k".to_string(), Column::from_utf8_contiguous(bytes, offs));
    cols.insert(
        "v".to_string(),
        Column::from_i64_values((0..n as i64).collect()),
    );
    let df = DataFrame::new_with_column_order(idx, cols, vec!["k".into(), "v".into()]).unwrap();
    let mut best = u128::MAX;
    for _ in 0..6 {
        let t = std::time::Instant::now();
        std::hint::black_box(df.explode("k", ",").unwrap().len());
        best = best.min(t.elapsed().as_nanos());
    }
    println!("explode n={n} (->{}): {:.2}ms", n * 3, best as f64 / 1e6);
}
