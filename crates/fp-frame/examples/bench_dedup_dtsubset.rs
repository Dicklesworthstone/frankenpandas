//! df.duplicated/drop_duplicates(subset=[i64 id, Datetime64 ts]) @1M.
//! Run: bench_dedup_dtsubset <n> <gcard> <op>
use std::collections::BTreeMap;

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::{DuplicateKeep, Index, IndexLabel};

fn sm(i: usize, s: u64) -> u64 {
    let mut h = (i as u64).wrapping_add(s).wrapping_mul(0x9E3779B97F4A7C15);
    h = (h ^ (h >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    h ^= h >> 31;
    h
}
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(1_000_000);
    let gc: u64 = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(100_000);
    let op = a.get(3).map(String::as_str).unwrap_or("drop");
    let base = 1_577_836_800_000_000_000i64;
    let day = 86_400_000_000_000i64;
    let id: Vec<i64> = (0..n).map(|i| (sm(i, 0) % gc) as i64).collect();
    let ts: Vec<i64> = (0..n)
        .map(|i| base + (sm(i, 1) % gc) as i64 * day)
        .collect();
    let labels: Vec<IndexLabel> = (0..n as i64).map(IndexLabel::Int64).collect();
    let mut cols = BTreeMap::new();
    cols.insert("id".to_string(), Column::from_i64_values(id));
    cols.insert("ts".to_string(), Column::from_datetime64_values(ts));
    let df =
        DataFrame::new_with_column_order(Index::new(labels), cols, vec!["id".into(), "ts".into()])
            .unwrap();
    let subset = vec!["id".to_string(), "ts".to_string()];
    let mut best = u128::MAX;
    for _ in 0..6 {
        let t = std::time::Instant::now();
        match op {
            "drop" => {
                std::hint::black_box(
                    df.drop_duplicates(Some(&subset), DuplicateKeep::First, false)
                        .unwrap(),
                );
            }
            "dup" => {
                std::hint::black_box(df.duplicated(Some(&subset), DuplicateKeep::First).unwrap());
            }
            _ => panic!("op"),
        };
        let e = t.elapsed().as_nanos();
        if e < best {
            best = e;
        }
    }
    println!("dedup_dtsubset_{op} n={n} gc={gc}: best={best}ns");
}
