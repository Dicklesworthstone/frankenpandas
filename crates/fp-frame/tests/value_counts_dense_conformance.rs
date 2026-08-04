//! No-mock conformance guard for value_counts' dense bounded-range Int64 histogram path
//! (br-frankenpandas-7c7d0) — a DISTINCT code path from the general FxHashMap path the
//! Utf8 guard (r5qtb) covers. Asserts the dense path yields the same count-desc /
//! first-seen-tiebreak ordering as the hashmap path. Compiled via `cargo check --tests`;
//! full run batch-pending.

use fp_frame::Series;
use fp_index::IndexLabel;
use fp_types::Scalar;

fn i64_series(vals: &[i64]) -> Series {
    Series::from_values(
        "v",
        (0..vals.len() as i64).map(IndexLabel::Int64).collect(),
        vals.iter().map(|&x| Scalar::Int64(x)).collect(),
    )
    .unwrap()
}

fn labels_i64(s: &Series) -> Vec<i64> {
    s.index()
        .labels()
        .iter()
        .map(|l| match l {
            IndexLabel::Int64(x) => *x,
            _ => i64::MIN,
        })
        .collect()
}

fn counts_i64(s: &Series) -> Vec<i64> {
    s.values()
        .iter()
        .map(|v| match v {
            Scalar::Int64(x) => *x,
            _ => i64::MIN,
        })
        .collect()
}

#[test]
fn value_counts_dense_int64_count_desc_first_seen() {
    // bounded range 1..=3, dense histogram path: 3 appears 3x, 1 twice, 2 once.
    // count-desc with first-seen tiebreak => [3, 1, 2].
    let vc = i64_series(&[3, 1, 3, 2, 3, 1]).value_counts().unwrap();
    assert_eq!(labels_i64(&vc), vec![3, 1, 2]);
    assert_eq!(counts_i64(&vc), vec![3, 2, 1]);
}

#[test]
fn value_counts_dense_int64_includes_negatives() {
    // span includes negative values (min offset) — still dense, still correct.
    let vc = i64_series(&[-2, 5, -2, 5, 5, 0]).value_counts().unwrap();
    assert_eq!(labels_i64(&vc), vec![5, -2, 0]);
    assert_eq!(counts_i64(&vc), vec![3, 2, 1]);
}

#[test]
fn value_counts_single_value() {
    let vc = i64_series(&[7, 7, 7]).value_counts().unwrap();
    assert_eq!(labels_i64(&vc), vec![7]);
    assert_eq!(counts_i64(&vc), vec![3]);
}
