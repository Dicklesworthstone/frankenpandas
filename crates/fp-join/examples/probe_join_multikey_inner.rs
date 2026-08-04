use std::{collections::BTreeMap, hint::black_box, time::Instant};

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::{Index, IndexLabel};
use fp_join::{JoinType, merge_dataframes_on_with};
fn build(n: usize) -> (DataFrame, DataFrame) {
    let idx = |m: usize| Index::new((0..m as i64).map(IndexLabel::Int64).collect());
    // UNIQUE composite (a,b): a=v%4, b=v/4 where v is a shuffle of 0..n -> (a,b) unique.
    // left v = perm1, right v = perm2 (half-overlap via *2 on right -> partial match).
    let lv: Vec<i64> = (0..n as i64)
        .map(|i| (i * 2654435761i64).rem_euclid(n as i64))
        .collect();
    let rv: Vec<i64> = (0..n as i64)
        .map(|i| ((i * 40503i64).rem_euclid(n as i64 / 2)) * 2)
        .collect(); // even values only -> ~half match, unique
    let la: Vec<i64> = lv.iter().map(|v| v % 4).collect();
    let lb: Vec<i64> = lv.iter().map(|v| v / 4).collect();
    let ra: Vec<i64> = rv.iter().map(|v| v % 4).collect();
    let rb: Vec<i64> = rv.iter().map(|v| v / 4).collect();
    let mut lm = BTreeMap::new();
    lm.insert("a".to_string(), Column::from_i64_values(la));
    lm.insert("b".to_string(), Column::from_i64_values(lb));
    lm.insert(
        "lv".to_string(),
        Column::from_f64_values((0..n).map(|i| i as f64).collect()),
    );
    let left =
        DataFrame::new_with_column_order(idx(n), lm, vec!["a".into(), "b".into(), "lv".into()])
            .unwrap();
    let mut rm = BTreeMap::new();
    rm.insert("a".to_string(), Column::from_i64_values(ra));
    rm.insert("b".to_string(), Column::from_i64_values(rb));
    rm.insert(
        "rv".to_string(),
        Column::from_f64_values((0..n).map(|i| i as f64 * 10.0).collect()),
    );
    let right =
        DataFrame::new_with_column_order(idx(n), rm, vec!["a".into(), "b".into(), "rv".into()])
            .unwrap();
    (left, right)
}
fn main() {
    let n = 1_000_000usize;
    let it = 20usize;
    let (l, r) = build(n);
    for _ in 0..3 {
        black_box(
            merge_dataframes_on_with(&l, &r, &["a", "b"], &["a", "b"], JoinType::Inner)
                .unwrap()
                .columns
                .len(),
        );
    }
    let st = Instant::now();
    let mut k = 0usize;
    for _ in 0..it {
        k ^= black_box(
            merge_dataframes_on_with(&l, &r, &["a", "b"], &["a", "b"], JoinType::Inner)
                .unwrap()
                .columns
                .len(),
        );
    }
    println!(
        "mkinner_uniq_1M        : {:.3} ms/call (k={k})",
        st.elapsed().as_secs_f64() * 1000.0 / it as f64
    );
}
