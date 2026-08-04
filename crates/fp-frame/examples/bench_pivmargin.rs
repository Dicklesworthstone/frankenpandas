//! pivot_table(margins=True). Run: -- 200000 200 10 8 sum
use std::{collections::BTreeMap, time::Instant};

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::{Index, IndexLabel};
use fp_types::Scalar;
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(200_000);
    let n_idx: i64 = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(200);
    let n_col: i64 = a.get(3).and_then(|s| s.parse().ok()).unwrap_or(10);
    let iters: usize = a.get(4).and_then(|s| s.parse().ok()).unwrap_or(8);
    let agg = a.get(5).map(String::as_str).unwrap_or("sum");
    let valtype = a.get(6).map(String::as_str).unwrap_or("f64");
    let index = Index::new((0..n as i64).map(IndexLabel::Int64).collect());
    let mut cols = BTreeMap::new();
    cols.insert(
        "idx".to_string(),
        Column::from_values(
            (0..n)
                .map(|i| Scalar::Int64(((i as i64).wrapping_mul(2654435761) >> 13) % n_idx))
                .collect(),
        )
        .unwrap(),
    );
    cols.insert(
        "col".to_string(),
        Column::from_values(
            (0..n)
                .map(|i| Scalar::Int64(((i as i64).wrapping_mul(40503) >> 7) % n_col))
                .collect(),
        )
        .unwrap(),
    );
    cols.insert(
        "val".to_string(),
        Column::from_values(
            (0..n)
                .map(|i| {
                    if valtype == "i64" {
                        Scalar::Int64((i % 997) as i64)
                    } else {
                        Scalar::Float64((i % 997) as f64 * 1.5)
                    }
                })
                .collect(),
        )
        .unwrap(),
    );
    let df = DataFrame::new_with_column_order(
        index,
        cols,
        vec!["idx".into(), "col".into(), "val".into()],
    )
    .unwrap();
    let mut best = u128::MAX;
    for _ in 0..iters {
        let t = Instant::now();
        std::hint::black_box(
            df.pivot_table_with_margins("val", "idx", "col", agg, true)
                .unwrap(),
        );
        let e = t.elapsed().as_nanos();
        if e < best {
            best = e;
        }
    }
    println!("pivot_margins n={n} n_idx={n_idx} n_col={n_col} agg={agg}: best={best}ns");
}
