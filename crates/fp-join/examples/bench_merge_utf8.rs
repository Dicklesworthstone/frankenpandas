//! Inner merge on a Utf8 key: 1M-row fact (card distinct keys) ⋈ card-row dim.
//! Run: cargo build --release -p fp-join --example bench_merge_utf8; bin 1000000 10000
use std::{collections::BTreeMap, hint::black_box, time::Instant};

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::{Index, IndexLabel};
use fp_join::{JoinType, merge_dataframes_on};
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(1_000_000);
    let card: i64 = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(10000);
    let it: usize = a.get(3).and_then(|s| s.parse().ok()).unwrap_or(10);
    let lidx = Index::new((0..n as i64).map(IndexLabel::Int64).collect());
    let mut lm = BTreeMap::new();
    lm.insert(
        "key".to_string(),
        Column::from_values(
            (0..n)
                .map(|i| {
                    fp_types::Scalar::Utf8(format!(
                        "k{:08}",
                        ((i as i64).wrapping_mul(2654435761) >> 13) % card
                    ))
                })
                .collect(),
        )
        .unwrap(),
    );
    lm.insert(
        "lv".to_string(),
        Column::from_f64_values((0..n).map(|i| i as f64).collect()),
    );
    let left = DataFrame::new_with_column_order(lidx, lm, vec!["key".into(), "lv".into()]).unwrap();
    let ridx = Index::new((0..card).map(IndexLabel::Int64).collect());
    let mut rm = BTreeMap::new();
    rm.insert(
        "key".to_string(),
        Column::from_values(
            (0..card)
                .map(|k| fp_types::Scalar::Utf8(format!("k{:08}", k)))
                .collect(),
        )
        .unwrap(),
    );
    rm.insert(
        "rv".to_string(),
        Column::from_f64_values((0..card).map(|k| k as f64 * 10.0).collect()),
    );
    let right =
        DataFrame::new_with_column_order(ridx, rm, vec!["key".into(), "rv".into()]).unwrap();
    let join_name = a.get(4).map(String::as_str).unwrap_or("inner");
    let jt = match join_name {
        "left" => JoinType::Left,
        "outer" => JoinType::Outer,
        "right" => JoinType::Right,
        _ => JoinType::Inner,
    };
    let on = ["key"];
    let mut best = u128::MAX;
    for _ in 0..it {
        let t = Instant::now();
        black_box(merge_dataframes_on(&left, &right, &on, jt).unwrap());
        let e = t.elapsed().as_nanos();
        if e < best {
            best = e;
        }
    }
    println!("merge_{join_name}_utf8 n={n} card={card}: best={best}ns");
}
