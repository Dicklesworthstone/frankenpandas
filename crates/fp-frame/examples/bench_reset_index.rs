//! Bench + golden for `DataFrame::reset_index(drop=false)` on a frame with an Int64
//! index — the flagship post-`groupby().agg()` pipeline step. Exercises the typed
//! Int64 index->column fast path (br-frankenpandas-bp6k7): the moved index column is
//! built via `Index::int64_label_values()` + `Column::from_i64_values` instead of a
//! per-label Scalar materialize + re-validation.
//! Golden = FNV-1a64 over the reset frame's first column (the former index) values.
//!
//! Run: cargo run -p fp-frame --example bench_reset_index --release -- 1000000 50

use std::{collections::BTreeMap, time::Instant};

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::{Index, IndexLabel};
use fp_types::Scalar;

fn build(n: usize) -> DataFrame {
    let index = Index::new((0..n as i64).map(IndexLabel::Int64).collect());
    let mut cols = BTreeMap::new();
    cols.insert(
        "a".to_string(),
        Column::from_values((0..n as i64).map(Scalar::Int64).collect()).unwrap(),
    );
    cols.insert(
        "b".to_string(),
        Column::from_values((0..n as i64).map(|v| Scalar::Int64(v * 2)).collect()).unwrap(),
    );
    DataFrame::new_with_column_order(index, cols, vec!["a".to_string(), "b".to_string()]).unwrap()
}

fn fnv1a64_update(h: &mut u64, bytes: &[u8]) {
    for b in bytes {
        *h ^= u64::from(*b);
        *h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
}

fn digest(df: &DataFrame) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    // Hash every column's values in column order so the moved-index column + data
    // are all pinned.
    for name in df.column_names() {
        fnv1a64_update(&mut h, name.as_bytes());
        if let Some(col) = df.column(name) {
            for v in col.values() {
                let x = match v {
                    Scalar::Int64(x) => *x,
                    Scalar::Float64(x) => *x as i64,
                    _ => i64::MIN,
                };
                fnv1a64_update(&mut h, &x.to_le_bytes());
            }
        }
    }
    h
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let n: usize = args
        .get(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(1_000_000);
    let iters: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(50);

    let df = build(n);
    let golden = digest(&df.reset_index(false).expect("reset_index"));

    let mut best = u128::MAX;
    for _ in 0..iters {
        let t = Instant::now();
        let out = df.reset_index(false).expect("reset_index");
        let e = t.elapsed().as_nanos();
        std::hint::black_box(&out);
        if e < best {
            best = e;
        }
    }

    println!("reset_index n={n} iters={iters}: best={best}ns golden={golden:016x}");
}
