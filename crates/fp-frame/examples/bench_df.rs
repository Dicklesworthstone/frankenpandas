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
    let min_axis1 = best(iters, || {
        std::hint::black_box(df.min_axis1().expect("min_axis1"));
    });
    let max_axis1 = best(iters, || {
        std::hint::black_box(df.max_axis1().expect("max_axis1"));
    });
    let prod_axis1 = best(iters, || {
        std::hint::black_box(df.prod_axis1().expect("prod_axis1"));
    });
    let mean_axis1 = best(iters, || {
        std::hint::black_box(df.mean_axis1().expect("mean_axis1"));
    });
    let var_axis1 = best(iters, || {
        std::hint::black_box(df.var_axis1().expect("var_axis1"));
    });
    let std_axis1 = best(iters, || {
        std::hint::black_box(df.std_axis1().expect("std_axis1"));
    });
    println!("df_axis1 min={min_axis1}ns max={max_axis1}ns prod={prod_axis1}ns mean={mean_axis1}ns var={var_axis1}ns std={std_axis1}ns");
    let sum_axis0 = best(iters, || {
        std::hint::black_box(df.sum().expect("sum"));
    });
    let std_axis0 = best(iters, || {
        std::hint::black_box(df.std().expect("std"));
    });
    let count_axis1 = best(iters, || {
        std::hint::black_box(df.count_axis1().expect("count_axis1"));
    });
    println!("df_axis0 sum={sum_axis0}ns std={std_axis0}ns count_axis1={count_axis1}ns");
    // transpose: small frame (transpose of 500k rows -> 500k cols is pathological; use a slice)
    let small = df.head(2000).expect("head");
    let transpose = best(iters, || {
        std::hint::black_box(small.transpose().expect("transpose"));
    });

    println!("df n={n} k={k}: sum_axis1={sum_axis1}ns transpose_2000x{k}={transpose}ns");
}
