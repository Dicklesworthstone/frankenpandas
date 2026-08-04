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
    let n: usize = 1_000_000;
    let g: u64 = 100;
    let index = Index::new_known_unique_int64_unit_range(0, n);
    let mut cols = BTreeMap::new();
    cols.insert(
        "k1".to_string(),
        Column::from_i64_values((0..n).map(|i| (sm(i, 0) % g) as i64).collect()),
    );
    cols.insert(
        "k2".to_string(),
        Column::from_i64_values((0..n).map(|i| (sm(i, 1) % g) as i64).collect()),
    );
    cols.insert(
        "a".to_string(),
        Column::from_i64_values((0..n).map(|i| (sm(i, 2) % 2000) as i64).collect()),
    );
    cols.insert(
        "b".to_string(),
        Column::from_i64_values((0..n).map(|i| (sm(i, 3) % 2000) as i64).collect()),
    );
    let df = DataFrame::new_with_column_order(
        index,
        cols,
        vec!["k1".into(), "k2".into(), "a".into(), "b".into()],
    )
    .unwrap();
    let mut best = u128::MAX;
    for _ in 0..6 {
        let t = Instant::now();
        std::hint::black_box(df.groupby(&["k1", "k2"]).unwrap().nunique().unwrap());
        let e = t.elapsed().as_nanos();
        if e < best {
            best = e;
        }
    }
    println!("nunique_i64 n={n} g={g}x{g}: best={best}ns");
}
