use std::{collections::BTreeMap, hint::black_box, time::Instant};

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::{Index, IndexLabel};
use fp_join::{JoinType, merge_dataframes_on};
fn build(n: usize) -> (DataFrame, DataFrame) {
    let idx = |m: usize| Index::new((0..m as i64).map(IndexLabel::Int64).collect());
    let mut lm = BTreeMap::new();
    lm.insert(
        "key".to_string(),
        Column::from_i64_values((0..n as i64).collect()),
    );
    lm.insert(
        "left_val".to_string(),
        Column::from_f64_values((0..n).map(|i| i as f64).collect()),
    );
    let left = DataFrame::new_with_column_order(idx(n), lm, vec!["key".into(), "left_val".into()])
        .unwrap();
    let mut rm = BTreeMap::new();
    rm.insert(
        "key".to_string(),
        Column::from_i64_values((0..n as i64).map(|i| i * 2).collect()),
    );
    rm.insert(
        "right_val".to_string(),
        Column::from_f64_values((0..n).map(|i| i as f64 * 10.0).collect()),
    );
    let right =
        DataFrame::new_with_column_order(idx(n), rm, vec!["key".into(), "right_val".into()])
            .unwrap();
    (left, right)
}
fn main() {
    let n = 1_000_000usize;
    let it = 30usize;
    // AMORTIZED (reused frames; .values() OnceLock cached after warmup)
    {
        let (l, r) = build(n);
        for _ in 0..3 {
            black_box(
                merge_dataframes_on(&l, &r, &["key"], JoinType::Left)
                    .unwrap()
                    .columns
                    .len(),
            );
        }
        let st = Instant::now();
        let mut k = 0usize;
        for _ in 0..it {
            k ^= black_box(
                merge_dataframes_on(&l, &r, &["key"], JoinType::Left)
                    .unwrap()
                    .columns
                    .len(),
            );
        }
        println!(
            "join_left_amortized    : {:.3} ms/call (k={k})",
            st.elapsed().as_secs_f64() * 1000.0 / it as f64
        );
    }
    // ONE-SHOT (fresh frames each iter; merge pays cold materialization on OLD)
    {
        let mut tot = 0.0;
        let mut k = 0usize;
        for _ in 0..3 {
            let (l, r) = build(n);
            black_box(
                merge_dataframes_on(&l, &r, &["key"], JoinType::Left)
                    .unwrap()
                    .columns
                    .len(),
            );
        }
        for _ in 0..it {
            let (l, r) = build(n);
            let st = Instant::now();
            k ^= black_box(
                merge_dataframes_on(&l, &r, &["key"], JoinType::Left)
                    .unwrap()
                    .columns
                    .len(),
            );
            tot += st.elapsed().as_secs_f64() * 1000.0;
        }
        println!(
            "join_left_oneshot      : {:.3} ms/call (k={k})",
            tot / it as f64
        );
    }
}
