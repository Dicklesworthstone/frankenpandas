//! DataFrame.where head-to-head. 500k×10 Float64, 50% bool cond frame, scalar other.
//! Verifies the where_mask_typed_f64 path. Run: ... bench_dfwhere_cc --release -- 500000 10 30

use std::{collections::BTreeMap, time::Instant};

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::{Index, IndexLabel};
use fp_types::Scalar;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let n: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(500_000);
    let k: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(10);
    let iters: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(30);
    let idx = Index::new((0..n as i64).map(IndexLabel::Int64).collect());
    let mut cols = BTreeMap::new();
    let mut ccols = BTreeMap::new();
    let mut order = Vec::new();
    for c in 0..k {
        let name = format!("c{c}");
        cols.insert(
            name.clone(),
            Column::from_values(
                (0..n)
                    .map(|i| Scalar::Float64((i + c) as f64 * 1.5))
                    .collect(),
            )
            .unwrap(),
        );
        ccols.insert(
            name.clone(),
            Column::from_values((0..n).map(|i| Scalar::Bool((i + c) % 2 == 0)).collect()).unwrap(),
        );
        order.push(name);
    }
    let df = DataFrame::new_with_column_order(idx.clone(), cols, order.clone()).unwrap();
    let cond = DataFrame::new_with_column_order(idx, ccols, order).unwrap();
    let other = Scalar::Float64(0.0);

    let mut best = u128::MAX;
    for _ in 0..iters {
        let t = Instant::now();
        let out = df.where_cond(&cond, Some(&other)).expect("where");
        let e = t.elapsed().as_nanos();
        std::hint::black_box(&out);
        if e < best {
            best = e;
        }
    }
    println!("df_where n={n} k={k}: best={best}ns");
}
