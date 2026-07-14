use std::{collections::BTreeMap, hint::black_box, time::Instant};

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::{Index, IndexLabel};
use fp_join::{JoinType, merge_dataframes_on};
use fp_types::Scalar;
fn col_dt(vals: Vec<i64>) -> Column {
    Column::from_values(vals.into_iter().map(Scalar::Datetime64).collect()).unwrap()
}
fn build(n: usize, datetime: bool) -> (DataFrame, DataFrame) {
    let idx = |m: usize| Index::new((0..m as i64).map(IndexLabel::Int64).collect());
    let lk: Vec<i64> = (0..n as i64).collect(); // ordered unique
    let rk: Vec<i64> = (0..n as i64).map(|i| i * 2).collect();
    let (lkc, rkc) = if datetime {
        (col_dt(lk), col_dt(rk))
    } else {
        (Column::from_i64_values(lk), Column::from_i64_values(rk))
    };
    let mut lm = BTreeMap::new();
    lm.insert("key".to_string(), lkc);
    lm.insert(
        "lv".to_string(),
        Column::from_f64_values((0..n).map(|i| i as f64).collect()),
    );
    let left =
        DataFrame::new_with_column_order(idx(n), lm, vec!["key".into(), "lv".into()]).unwrap();
    let mut rm = BTreeMap::new();
    rm.insert("key".to_string(), rkc);
    rm.insert(
        "rv".to_string(),
        Column::from_f64_values((0..n).map(|i| i as f64 * 10.0).collect()),
    );
    let right =
        DataFrame::new_with_column_order(idx(n), rm, vec!["key".into(), "rv".into()]).unwrap();
    (left, right)
}
fn bench(name: &str, l: &DataFrame, r: &DataFrame, expected_rows: usize, it: usize) {
    for _ in 0..2 {
        black_box(
            merge_dataframes_on(l, r, &["key"], JoinType::Inner)
                .unwrap()
                .index
                .len(),
        );
    }
    let mut samples = Vec::with_capacity(it);
    let mut checksum = 0usize;
    for _ in 0..it {
        let start = Instant::now();
        let rows = black_box(
            merge_dataframes_on(l, r, &["key"], JoinType::Inner)
                .unwrap()
                .index
                .len(),
        );
        assert_eq!(rows, expected_rows);
        checksum ^= rows;
        samples.push(start.elapsed().as_secs_f64() * 1_000.0);
    }
    samples.sort_by(f64::total_cmp);
    println!(
        "{name:28}: p50={:.3} ms min={:.3} ms (rows={expected_rows}, checksum={checksum})",
        samples[samples.len() / 2],
        samples[0]
    );
}
fn main() {
    let n = 200_000usize;
    let expected_rows = n.div_ceil(2);
    let (li, ri) = build(n, false);
    bench("inner_int64_control_a", &li, &ri, expected_rows, 7);
    bench("inner_int64_control_b", &li, &ri, expected_rows, 7);
    let (ld, rd) = build(n, true);
    bench("inner_datetime_key_a", &ld, &rd, expected_rows, 7);
    bench("inner_datetime_key_b", &ld, &rd, expected_rows, 7);
}
