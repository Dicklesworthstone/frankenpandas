//! DataFrame.get_dummies head-to-head. n rows, one Utf8 col with k categories.
//! Run: cargo run -p fp-frame --example bench_dummies_cc --release -- 1000000 30 20
use std::{collections::BTreeMap, time::Instant};

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::{Index, IndexLabel};
use fp_types::Scalar;
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(1_000_000);
    let k: i64 = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(30);
    let iters: usize = a.get(3).and_then(|s| s.parse().ok()).unwrap_or(20);
    let index = Index::new((0..n as i64).map(IndexLabel::Int64).collect());
    let mut cols = BTreeMap::new();
    cols.insert(
        "cat".to_string(),
        Column::from_values(
            (0..n)
                .map(|i| {
                    Scalar::Utf8(format!(
                        "cat{:03}",
                        ((i as i64).wrapping_mul(2654435761) >> 13) % k
                    ))
                })
                .collect(),
        )
        .unwrap(),
    );
    let df = DataFrame::new_with_column_order(index, cols, vec!["cat".to_string()]).unwrap();
    let mut best = u128::MAX;
    for _ in 0..iters {
        let t = Instant::now();
        std::hint::black_box(df.get_dummies(&["cat"]).unwrap());
        let e = t.elapsed().as_nanos();
        if e < best {
            best = e;
        }
    }
    println!("get_dummies n={n} k={k}: best={best}ns");
}
