//! Bench for DataFrame::set_index typed Int64 column→index (p9omo:
//! Index::from_i64_values). Sets an Int64 column as the index on a 1M-row frame.
//! Compare vs pandas df.set_index('a').
//!
//! Run: cargo run -p fp-frame --example bench_set_index --release -- 1000000 50

use std::collections::BTreeMap;
use std::time::Instant;

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::{Index, IndexLabel};
use fp_types::Scalar;

fn build(n: usize) -> DataFrame {
    let index = Index::new((0..n as i64).map(IndexLabel::Int64).collect());
    let mut cols = BTreeMap::new();
    cols.insert(
        "a".to_string(),
        Column::from_values((0..n as i64).map(|x| Scalar::Int64(x * 3)).collect()).unwrap(),
    );
    cols.insert(
        "b".to_string(),
        Column::from_values((0..n as i64).map(Scalar::Int64).collect()).unwrap(),
    );
    DataFrame::new_with_column_order(index, cols, vec!["a".to_string(), "b".to_string()]).unwrap()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let n: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(1_000_000);
    let iters: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(50);
    let df = build(n);
    let mut best = u128::MAX;
    for _ in 0..iters {
        let t = Instant::now();
        let out = df.set_index("a", false).expect("set_index");
        let e = t.elapsed().as_nanos();
        std::hint::black_box(&out);
        if e < best {
            best = e;
        }
    }
    println!("set_index n={n} iters={iters}: best={best}ns");
}
