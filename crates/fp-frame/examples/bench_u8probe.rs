//! Common ops on Scalar-backed Utf8 (pivot_table/map/replace). bench_u8probe <n> <card>
use std::collections::BTreeMap;

use fp_columnar::Column;
use fp_frame::{DataFrame, Series};
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
        .unwrap_or(1_000_000);
    let icard: usize = std::env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(1000);
    let ccard: usize = 20;
    let icats: Vec<String> = (0..icard).map(|c| format!("idx_{c:05}")).collect();
    let ccats: Vec<String> = (0..ccard).map(|c| format!("col_{c:03}")).collect();
    let labels: Vec<IndexLabel> = (0..n as i64).map(IndexLabel::Int64).collect();
    // pivot_table with Scalar-backed Utf8 index + column
    let mut cols = BTreeMap::new();
    cols.insert(
        "i".to_string(),
        Column::from_values(
            (0..n)
                .map(|x| Scalar::Utf8(icats[(sm(x, 0) as usize) % icard].clone()))
                .collect(),
        )
        .unwrap(),
    );
    cols.insert(
        "c".to_string(),
        Column::from_values(
            (0..n)
                .map(|x| Scalar::Utf8(ccats[(sm(x, 1) as usize) % ccard].clone()))
                .collect(),
        )
        .unwrap(),
    );
    cols.insert(
        "v".to_string(),
        Column::from_f64_values((0..n).map(|x| sm(x, 2) as f64).collect()),
    );
    let df = DataFrame::new_with_column_order(
        Index::new(labels.clone()),
        cols,
        vec!["i".into(), "c".into(), "v".into()],
    )
    .unwrap();
    timeit("pivot_table_sum", || {
        std::hint::black_box(df.pivot_table("v", "i", "c", "sum").unwrap().shape());
    });
    // map dict on Utf8
    let su = Series::from_values(
        "s",
        labels.clone(),
        (0..n)
            .map(|x| Scalar::Utf8(icats[(sm(x, 0) as usize) % icard].clone()))
            .collect(),
    )
    .unwrap();
    let mapping: Vec<(Scalar, Scalar)> = (0..icard)
        .map(|c| (Scalar::Utf8(icats[c].clone()), Scalar::Int64(c as i64)))
        .collect();
    timeit("map_utf8_to_i64", || {
        std::hint::black_box(su.map(&mapping).unwrap().len());
    });
    timeit("replace_utf8", || {
        std::hint::black_box(su.replace(&mapping).unwrap().len());
    });
}
