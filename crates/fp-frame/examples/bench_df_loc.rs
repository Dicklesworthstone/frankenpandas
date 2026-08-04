//! Bench + golden digest for DataFrame::loc(&[IndexLabel]) (df.loc[[...]]).
//!
//! Run: cargo run -p fp-frame --example bench_df_loc --release
//!
//! loc_with_columns rescanned the whole row index once PER requested label =
//! O(m·n). A label->positions multimap built once makes it O(m+n). Duplicate
//! index labels return all matches in ascending index order; selector order
//! and duplicate selectors are preserved; a missing label fails closed.

use std::{collections::BTreeMap, time::Instant};

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::{Index, IndexLabel};
use fp_types::Scalar;

fn df_from(labels: Vec<i64>, c0: Vec<i64>, c1: Vec<i64>) -> DataFrame {
    let index = Index::new(labels.into_iter().map(IndexLabel::Int64).collect());
    let mut cols = BTreeMap::new();
    cols.insert(
        "a".to_string(),
        Column::from_values(c0.into_iter().map(Scalar::Int64).collect()).unwrap(),
    );
    cols.insert(
        "b".to_string(),
        Column::from_values(c1.into_iter().map(Scalar::Int64).collect()).unwrap(),
    );
    DataFrame::new_with_column_order(index, cols, vec!["a".to_string(), "b".to_string()]).unwrap()
}

fn golden() -> String {
    let mut out = String::new();
    // Duplicate index label 10 at positions 0 and 2.
    let df = df_from(
        vec![10, 20, 10, 30],
        vec![100, 200, 300, 400],
        vec![1, 2, 3, 4],
    );
    // Selector order [30, 10, 20]; 10 returns both matches (ascending position).
    let r = df
        .loc(&[
            IndexLabel::Int64(30),
            IndexLabel::Int64(10),
            IndexLabel::Int64(20),
        ])
        .unwrap();
    out.push_str(&format!("labels={:?}\n", r.index().labels()));
    out.push_str(&format!("a={:?}\n", r.columns().get("a").unwrap().values()));
    out.push_str(&format!("b={:?}\n", r.columns().get("b").unwrap().values()));
    // Duplicate selector entries preserved.
    let r2 = df
        .loc(&[IndexLabel::Int64(20), IndexLabel::Int64(20)])
        .unwrap();
    out.push_str(&format!(
        "dup_a={:?}\n",
        r2.columns().get("a").unwrap().values()
    ));
    // Missing label fails closed.
    let err = df.loc(&[IndexLabel::Int64(99)]);
    out.push_str(&format!("missing_is_err={}\n", err.is_err()));
    out
}

fn main() {
    let g = golden();
    print!("GOLDEN_BEGIN\n{g}GOLDEN_END\n");

    let n: usize = 60_000;
    let labels: Vec<i64> = (0..n as i64).collect();
    let df = df_from(
        labels.clone(),
        (0..n as i64).map(|v| v * 2).collect(),
        (0..n as i64).map(|v| v * 3).collect(),
    );
    let selector: Vec<IndexLabel> = (0..n as i64).map(IndexLabel::Int64).collect();

    // warmup
    let _ = df.loc(&selector).unwrap();

    let t = Instant::now();
    let r = df.loc(&selector).unwrap();
    let d = t.elapsed();
    assert_eq!(r.len(), n);

    println!("TIMING n={n} m={n} df_loc={:.3}ms", d.as_secs_f64() * 1e3);
}
