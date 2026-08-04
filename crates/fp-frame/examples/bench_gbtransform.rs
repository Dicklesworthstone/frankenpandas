//! DataFrameGroupBy.transform('mean') on a Utf8 key. Run: -- 1000000 1000 12
use std::{collections::BTreeMap, time::Instant};

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::{Index, IndexLabel};
use fp_types::Scalar;
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(1_000_000);
    let card: i64 = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(1000);
    let iters: usize = a.get(3).and_then(|s| s.parse().ok()).unwrap_or(12);
    let func = a.get(4).map(String::as_str).unwrap_or("mean");
    let index = Index::new((0..n as i64).map(IndexLabel::Int64).collect());
    let mut cols = BTreeMap::new();
    cols.insert(
        "k".to_string(),
        Column::from_values(
            (0..n)
                .map(|i| {
                    Scalar::Utf8(format!(
                        "k{:04}",
                        ((i as i64).wrapping_mul(2654435761) >> 13) % card
                    ))
                })
                .collect(),
        )
        .unwrap(),
    );
    cols.insert(
        "v".to_string(),
        Column::from_values(
            (0..n)
                .map(|i| Scalar::Float64((i % 997) as f64 * 1.5))
                .collect(),
        )
        .unwrap(),
    );
    let df = DataFrame::new_with_column_order(index, cols, vec!["k".into(), "v".into()]).unwrap();
    let mut best = u128::MAX;
    for _ in 0..iters {
        let t = Instant::now();
        std::hint::black_box(df.groupby(&["k"]).unwrap().transform(func).unwrap());
        let e = t.elapsed().as_nanos();
        if e < best {
            best = e;
        }
    }
    println!("gb_transform_{func}_utf8 n={n} card={card}: best={best}ns");
}
