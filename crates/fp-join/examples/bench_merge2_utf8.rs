//! Inner/left/outer merge on TWO Utf8 keys (composite path) @1M.
//! Run: bench_merge2_utf8 <n> <card> <iters> <how>
use std::{collections::BTreeMap, hint::black_box, time::Instant};

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::{Index, IndexLabel};
use fp_join::{JoinType, merge_dataframes_on};

fn contig(n: usize, f: impl Fn(usize) -> String) -> Column {
    let mut bytes = Vec::new();
    let mut offsets = Vec::with_capacity(n + 1);
    offsets.push(0usize);
    for i in 0..n {
        bytes.extend_from_slice(f(i).as_bytes());
        offsets.push(bytes.len());
    }
    Column::from_utf8_contiguous(bytes, offsets)
}
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(1_000_000);
    let card: i64 = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(1000);
    let it: usize = a.get(3).and_then(|s| s.parse().ok()).unwrap_or(10);
    let how = a.get(4).map(String::as_str).unwrap_or("inner");
    let key = |i: usize, salt: i64| ((i as i64).wrapping_mul(2654435761).wrapping_add(salt) >> 13) % card;
    // left: 1M fact, (k1,k2)
    let lidx = Index::new((0..n as i64).map(IndexLabel::Int64).collect());
    let mut lm = BTreeMap::new();
    lm.insert("k1".to_string(), contig(n, |i| format!("a{:05}", key(i, 0))));
    lm.insert("k2".to_string(), contig(n, |i| format!("b{:05}", key(i, 7))));
    lm.insert(
        "lv".to_string(),
        Column::from_f64_values((0..n).map(|i| i as f64).collect()),
    );
    let left = DataFrame::new_with_column_order(lidx, lm, vec!["k1".into(), "k2".into(), "lv".into()]).unwrap();
    // right: card*card dim, every (k1,k2) combo once
    let m = (card * card) as usize;
    let ridx = Index::new((0..m as i64).map(IndexLabel::Int64).collect());
    let mut rm = BTreeMap::new();
    rm.insert("k1".to_string(), contig(m, |i| format!("a{:05}", (i as i64) / card)));
    rm.insert("k2".to_string(), contig(m, |i| format!("b{:05}", (i as i64) % card)));
    rm.insert(
        "rv".to_string(),
        Column::from_f64_values((0..m).map(|i| i as f64 * 2.0).collect()),
    );
    let right = DataFrame::new_with_column_order(ridx, rm, vec!["k1".into(), "k2".into(), "rv".into()]).unwrap();
    let jt = match how {
        "left" => JoinType::Left,
        "outer" => JoinType::Outer,
        "right" => JoinType::Right,
        _ => JoinType::Inner,
    };
    let on = ["k1", "k2"];
    let mut best = u128::MAX;
    for _ in 0..it {
        let t = Instant::now();
        black_box(merge_dataframes_on(&left, &right, &on, jt).unwrap());
        let e = t.elapsed().as_nanos();
        if e < best {
            best = e;
        }
    }
    println!("merge2_{how}_utf8 n={n} card={card}: best={best}ns");
}
