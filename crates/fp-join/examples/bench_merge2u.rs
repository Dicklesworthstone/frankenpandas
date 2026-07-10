//! merge on TWO Scalar-backed Utf8 keys (from_values). bench_merge2u <n> <card> <how>
use std::{collections::BTreeMap, hint::black_box, time::Instant};

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::{Index, IndexLabel};
use fp_join::{JoinType, merge_dataframes_on};
use fp_types::Scalar;
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(2_000_000);
    let card: i64 = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(1000);
    let how = a.get(3).map(String::as_str).unwrap_or("inner");
    let key =
        |i: usize, salt: i64| ((i as i64).wrapping_mul(2654435761).wrapping_add(salt) >> 13) % card;
    let scol = |rows: usize, f: &dyn Fn(usize) -> String| {
        Column::from_values((0..rows).map(|i| Scalar::Utf8(f(i))).collect()).unwrap()
    };
    let mut lm = BTreeMap::new();
    lm.insert("k1".to_string(), scol(n, &|i| format!("a{:05}", key(i, 0))));
    lm.insert("k2".to_string(), scol(n, &|i| format!("b{:05}", key(i, 7))));
    lm.insert(
        "lv".to_string(),
        Column::from_f64_values((0..n).map(|i| i as f64).collect()),
    );
    let left = DataFrame::new_with_column_order(
        Index::new((0..n as i64).map(IndexLabel::Int64).collect()),
        lm,
        vec!["k1".into(), "k2".into(), "lv".into()],
    )
    .unwrap();
    let m = (card * card) as usize;
    let mut rm = BTreeMap::new();
    rm.insert(
        "k1".to_string(),
        scol(m, &|i| format!("a{:05}", (i as i64) / card)),
    );
    rm.insert(
        "k2".to_string(),
        scol(m, &|i| format!("b{:05}", (i as i64) % card)),
    );
    rm.insert(
        "rv".to_string(),
        Column::from_f64_values((0..m).map(|i| i as f64).collect()),
    );
    let right = DataFrame::new_with_column_order(
        Index::new((0..m as i64).map(IndexLabel::Int64).collect()),
        rm,
        vec!["k1".into(), "k2".into(), "rv".into()],
    )
    .unwrap();
    let jt = match how {
        "left" => JoinType::Left,
        "outer" => JoinType::Outer,
        "right" => JoinType::Right,
        _ => JoinType::Inner,
    };
    let on = ["k1", "k2"];
    let mut best = u128::MAX;
    for _ in 0..6 {
        let t = Instant::now();
        black_box(merge_dataframes_on(&left, &right, &on, jt).unwrap());
        best = best.min(t.elapsed().as_nanos());
    }
    println!(
        "merge2u_{how} n={n} card={card}: {:.2}ms",
        best as f64 / 1e6
    );

    // PROTOTYPE of the dense_packed lever: factorize each Utf8 key column over
    // left+right to shared i64 codes, then merge on i64-code key columns (the
    // existing dense Int64 composite CSR). Measures whether the lever would pay
    // BEFORE writing the intricate in-place version. Output gather is unchanged
    // (this only times the matching cost difference vs the Utf8 composite path).
    use std::collections::HashMap;
    let code_col = |strs1: &[String], strs2: &[String]| -> (Vec<i64>, Vec<i64>) {
        let mut map: HashMap<&str, i64> = HashMap::new();
        let mut next = 0i64;
        let mut c1 = Vec::with_capacity(strs1.len());
        let mut c2 = Vec::with_capacity(strs2.len());
        for s in strs1 {
            let c = *map.entry(s).or_insert_with(|| {
                let x = next;
                next += 1;
                x
            });
            c1.push(c);
        }
        for s in strs2 {
            let c = *map.entry(s).or_insert_with(|| {
                let x = next;
                next += 1;
                x
            });
            c2.push(c);
        }
        (c1, c2)
    };
    let lk1: Vec<String> = (0..n).map(|i| format!("a{:05}", key(i, 0))).collect();
    let rk1: Vec<String> = (0..m)
        .map(|i| format!("a{:05}", (i as i64) / card))
        .collect();
    let lk2: Vec<String> = (0..n).map(|i| format!("b{:05}", key(i, 7))).collect();
    let rk2: Vec<String> = (0..m)
        .map(|i| format!("b{:05}", (i as i64) % card))
        .collect();
    let (lc1, rc1) = code_col(&lk1, &rk1);
    let (lc2, rc2) = code_col(&lk2, &rk2);
    let mut lcm = BTreeMap::new();
    lcm.insert("k1".to_string(), Column::from_i64_values(lc1));
    lcm.insert("k2".to_string(), Column::from_i64_values(lc2));
    lcm.insert(
        "lv".to_string(),
        Column::from_f64_values((0..n).map(|i| i as f64).collect()),
    );
    let leftc = DataFrame::new_with_column_order(
        Index::new((0..n as i64).map(IndexLabel::Int64).collect()),
        lcm,
        vec!["k1".into(), "k2".into(), "lv".into()],
    )
    .unwrap();
    let mut rcm = BTreeMap::new();
    rcm.insert("k1".to_string(), Column::from_i64_values(rc1));
    rcm.insert("k2".to_string(), Column::from_i64_values(rc2));
    rcm.insert(
        "rv".to_string(),
        Column::from_f64_values((0..m).map(|i| i as f64).collect()),
    );
    let rightc = DataFrame::new_with_column_order(
        Index::new((0..m as i64).map(IndexLabel::Int64).collect()),
        rcm,
        vec!["k1".into(), "k2".into(), "rv".into()],
    )
    .unwrap();
    let mut bestc = u128::MAX;
    for _ in 0..6 {
        let t = Instant::now();
        black_box(merge_dataframes_on(&leftc, &rightc, &on, jt).unwrap());
        bestc = bestc.min(t.elapsed().as_nanos());
    }
    println!(
        "merge2u_{how}_CODED n={n} card={card}: {:.2}ms",
        bestc as f64 / 1e6
    );
}
