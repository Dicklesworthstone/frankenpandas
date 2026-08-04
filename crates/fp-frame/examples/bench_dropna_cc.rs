//! DataFrame.dropna head-to-head (rows with any NaN dropped). 500k×5 Float64, ~10% rows
//! carry a NaN. Isolated bench (unique name; committed standalone during a hot shared-tree
//! window to avoid conflicting with peers' bench files).
//!
//! Run: cargo run -p fp-frame --example bench_dropna_cc --release -- 500000 5 30

use std::{collections::BTreeMap, time::Instant};

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::{Index, IndexLabel};
use fp_types::Scalar;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let n: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(500_000);
    let k: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(5);
    let iters: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(30);
    let index = Index::new((0..n as i64).map(IndexLabel::Int64).collect());
    let mut cols = BTreeMap::new();
    let mut order = Vec::new();
    for c in 0..k {
        let name = format!("c{c}");
        let vals: Vec<Scalar> = (0..n)
            .map(|i| {
                if c == 0 && i % 10 == 7 {
                    Scalar::Float64(f64::NAN)
                } else {
                    Scalar::Float64((i + c) as f64 * 1.5)
                }
            })
            .collect();
        cols.insert(name.clone(), Column::from_values(vals).unwrap());
        order.push(name);
    }
    let df = DataFrame::new_with_column_order(index, cols, order).unwrap();

    let mut best = u128::MAX;
    for _ in 0..iters {
        let t = Instant::now();
        let out = df.dropna().expect("dropna");
        let e = t.elapsed().as_nanos();
        std::hint::black_box(&out);
        if e < best {
            best = e;
        }
    }
    println!("dropna n={n} k={k}: best={best}ns");
}
