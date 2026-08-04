use std::{collections::BTreeMap, time::Instant};

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::Index;
fn sm(i: usize, s: u64) -> u64 {
    let mut h = (i as u64).wrapping_add(s).wrapping_mul(0x9E3779B97F4A7C15);
    h = (h ^ (h >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    h ^= h >> 31;
    h
}
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let nr: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(200);
    let nc: usize = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(200);
    let it: usize = a.get(3).and_then(|s| s.parse().ok()).unwrap_or(20);
    let n = nr * nc;
    let index = Index::new_known_unique_int64_unit_range(0, n);
    let mut cols = BTreeMap::new();
    cols.insert(
        "r".to_string(),
        Column::from_i64_values((0..n).map(|i| (i / nc) as i64).collect()),
    );
    cols.insert(
        "c".to_string(),
        Column::from_i64_values((0..n).map(|i| (i % nc) as i64).collect()),
    );
    cols.insert(
        "v".to_string(),
        Column::from_f64_values((0..n).map(|i| sm(i, 1) as f64).collect()),
    );
    let df =
        DataFrame::new_with_column_order(index, cols, vec!["r".into(), "c".into(), "v".into()])
            .unwrap();
    let mut best = u128::MAX;
    for _ in 0..it {
        let t = Instant::now();
        std::hint::black_box(df.pivot("r", "c", "v").unwrap());
        let e = t.elapsed().as_nanos();
        if e < best {
            best = e;
        }
    }
    println!("pivot {nr}x{nc}: best={best}ns");
}
