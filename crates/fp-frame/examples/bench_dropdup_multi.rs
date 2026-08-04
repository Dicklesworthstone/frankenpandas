//! DataFrame.drop_duplicates / duplicated on TWO contiguous-Utf8 columns @1M.
//! Run: bench_dropdup_multi <n> <card> <op>
use std::{collections::BTreeMap, time::Instant};

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::{DuplicateKeep, Index};

fn sm(i: usize, s: u64) -> u64 {
    let mut h = (i as u64).wrapping_add(s).wrapping_mul(0x9E3779B97F4A7C15);
    h = (h ^ (h >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    h ^= h >> 31;
    h
}
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
    let card: u64 = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(100);
    let op = a.get(3).map(String::as_str).unwrap_or("drop");
    let index = Index::new_known_unique_int64_unit_range(0, n);
    let mut cols = BTreeMap::new();
    cols.insert(
        "k1".to_string(),
        contig(n, |i| format!("g{:04}", sm(i, 0) % card)),
    );
    cols.insert(
        "k2".to_string(),
        contig(n, |i| format!("h{:04}", sm(i, 1) % card)),
    );
    let df = DataFrame::new_with_column_order(index, cols, vec!["k1".into(), "k2".into()]).unwrap();
    let subset = vec!["k1".to_string(), "k2".to_string()];
    let mut best = u128::MAX;
    for _ in 0..6 {
        let t = Instant::now();
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
        }
        let e = t.elapsed().as_nanos();
        if e < best {
            best = e;
        }
    }
    println!("dropdup2_{op} n={n} card={card}: best={best}ns");
}
