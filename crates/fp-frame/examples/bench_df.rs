//! DataFrame-level sweep: sum(axis=1) row-wise + transpose. Float64 frame.
//! Run: cargo run -p fp-frame --example bench_df --release -- 500000 10 30

use std::collections::BTreeMap;
use std::time::Instant;

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::{Index, IndexLabel};
use fp_types::Scalar;

fn best<F: FnMut()>(iters: usize, mut f: F) -> u128 {
    let mut b = u128::MAX;
    for _ in 0..iters {
        let t = Instant::now();
        f();
        let e = t.elapsed().as_nanos();
        if e < b {
            b = e;
        }
    }
    b
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let n: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(500_000);
    let k: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(10);
    let iters: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(30);
    let index = Index::new((0..n as i64).map(IndexLabel::Int64).collect());
    let mut cols = BTreeMap::new();
    let mut order = Vec::new();
    for c in 0..k {
        let name = format!("c{c}");
        cols.insert(
            name.clone(),
            Column::from_values((0..n).map(|i| Scalar::Float64((i + c) as f64 * 1.5)).collect())
                .unwrap(),
        );
        order.push(name);
    }
    let df = DataFrame::new_with_column_order(index, cols, order).unwrap();

    let sum_axis1 = best(iters, || {
        std::hint::black_box(df.sum_axis1().expect("sum_axis1"));
    });
    // transpose: small frame (transpose of 500k rows -> 500k cols is pathological; use a slice)
    let small = df.head(2000).expect("head");
    let transpose = best(iters, || {
        std::hint::black_box(small.transpose().expect("transpose"));
    });

    println!("df n={n} k={k}: sum_axis1={sum_axis1}ns transpose_2000x{k}={transpose}ns");
}
