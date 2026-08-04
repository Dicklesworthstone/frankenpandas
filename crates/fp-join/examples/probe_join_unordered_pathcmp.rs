//! A/B the same logical unordered int64 inner join routed through the
//! counting-sort dense path (keys in [0,n), span=n <= 4*rows) vs the generic
//! hash path (keys scaled x5 so span=5n > 4*rows -> dense bails). Identical
//! match structure, different code path — isolates counting-sort vs hash cost.
use std::{collections::BTreeMap, hint::black_box, time::Instant};

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::{Index, IndexLabel};
use fp_join::{JoinType, merge_dataframes_on};
fn build(n: usize, scale: i64) -> (DataFrame, DataFrame) {
    let idx = |m: usize| Index::new((0..m as i64).map(IndexLabel::Int64).collect());
    let lk: Vec<i64> = (0..n as i64)
        .map(|i| ((i * 2654435761i64).rem_euclid(n as i64)) * scale)
        .collect();
    let rk: Vec<i64> = (0..n as i64)
        .map(|i| ((i * 7) % (n as i64)) * scale)
        .collect();
    let mut lm = BTreeMap::new();
    lm.insert("key".to_string(), Column::from_i64_values(lk));
    lm.insert(
        "left_val".to_string(),
        Column::from_f64_values((0..n).map(|i| i as f64).collect()),
    );
    let left = DataFrame::new_with_column_order(idx(n), lm, vec!["key".into(), "left_val".into()])
        .unwrap();
    let mut rm = BTreeMap::new();
    rm.insert("key".to_string(), Column::from_i64_values(rk));
    rm.insert(
        "right_val".to_string(),
        Column::from_f64_values((0..n).map(|i| i as f64 * 10.0).collect()),
    );
    let right =
        DataFrame::new_with_column_order(idx(n), rm, vec!["key".into(), "right_val".into()])
            .unwrap();
    (left, right)
}
fn bench(name: &str, l: &DataFrame, r: &DataFrame, it: usize) {
    for _ in 0..3 {
        black_box(
            merge_dataframes_on(l, r, &["key"], JoinType::Inner)
                .unwrap()
                .columns
                .len(),
        );
    }
    let st = Instant::now();
    let mut k = 0usize;
    for _ in 0..it {
        k ^= black_box(
            merge_dataframes_on(l, r, &["key"], JoinType::Inner)
                .unwrap()
                .columns
                .len(),
        );
    }
    println!(
        "{name:34}: {:.3} ms/call (k={k})",
        st.elapsed().as_secs_f64() * 1000.0 / it as f64
    );
}
fn main() {
    let n = 1_000_000usize;
    let (la, ra) = build(n, 1); // span=n -> counting-sort dense path
    let (lb, rb) = build(n, 5); // span=5n>4n -> generic hash path
    bench("inner_span_n_countingsort", &la, &ra, 30);
    bench("inner_span_5n_hashpath", &lb, &rb, 30);
    // also low-cardinality: 1000 distinct keys (span small -> counting sort ideal)
    let mut z = 0x9u64;
    let idx = Index::new((0..n as i64).map(IndexLabel::Int64).collect());
    let lk: Vec<i64> = (0..n)
        .map(|_| {
            z ^= z << 13;
            z ^= z >> 7;
            z ^= z << 17;
            (z % 1000) as i64
        })
        .collect();
    let rk: Vec<i64> = (0..n)
        .map(|_| {
            z ^= z << 13;
            z ^= z >> 7;
            z ^= z << 17;
            (z % 1000) as i64
        })
        .collect();
    let mut lm = BTreeMap::new();
    lm.insert("key".to_string(), Column::from_i64_values(lk));
    lm.insert(
        "left_val".to_string(),
        Column::from_f64_values(vec![1.0; n]),
    );
    let llc =
        DataFrame::new_with_column_order(idx.clone(), lm, vec!["key".into(), "left_val".into()])
            .unwrap();
    let mut rm = BTreeMap::new();
    rm.insert("key".to_string(), Column::from_i64_values(rk));
    rm.insert(
        "right_val".to_string(),
        Column::from_f64_values(vec![2.0; n]),
    );
    let rlc =
        DataFrame::new_with_column_order(idx, rm, vec!["key".into(), "right_val".into()]).unwrap();
    bench("inner_lowcard_1000_countingsort", &llc, &rlc, 5);
}
