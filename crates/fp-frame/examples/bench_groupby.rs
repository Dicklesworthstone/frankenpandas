//! Bench for DataFrameGroupBy.sum on an Int64 key (build_groups dense/FxHash, buguz) and a
//! Utf8 key (FxHash general path). n rows over `card` distinct keys. Compare vs pandas
//! df.groupby(k).sum().
//!
//! Run: cargo run -p fp-frame --example bench_groupby --release -- 1000000 1000 20

use std::{collections::BTreeMap, time::Instant};

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::{Index, IndexLabel};
use fp_types::Scalar;

fn build(n: usize, card: usize, utf8_key: bool) -> DataFrame {
    let index = Index::new((0..n as i64).map(IndexLabel::Int64).collect());
    let mut cols = BTreeMap::new();
    let key_col = if utf8_key {
        Column::from_values(
            (0..n)
                .map(|i| Scalar::Utf8(format!("k{:04}", i % card)))
                .collect(),
        )
        .unwrap()
    } else {
        Column::from_values((0..n).map(|i| Scalar::Int64((i % card) as i64)).collect()).unwrap()
    };
    cols.insert("k".to_string(), key_col);
    cols.insert(
        "v".to_string(),
        Column::from_values((0..n as i64).map(Scalar::Int64).collect()).unwrap(),
    );
    DataFrame::new_with_column_order(index, cols, vec!["k".to_string(), "v".to_string()]).unwrap()
}

fn best<F: Fn()>(iters: usize, f: F) -> u128 {
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
    let n: usize = args
        .get(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(1_000_000);
    let card: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(1000);
    let iters: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(20);

    let df_i = build(n, card, false);
    let df_s = build(n, card, true);
    let int_sum = best(iters, || {
        std::hint::black_box(df_i.groupby(&["k"]).unwrap().sum().unwrap());
    });
    let utf8_sum = best(iters, || {
        std::hint::black_box(df_s.groupby(&["k"]).unwrap().sum().unwrap());
    });
    println!("groupby_sum n={n} card={card}: int_key={int_sum}ns utf8_key={utf8_sum}ns");
}
