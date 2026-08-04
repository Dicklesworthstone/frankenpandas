//! DataFrame.loc[[timedelta labels]] over a unique Timedelta64 index.
//! Run: bench_td_loc <n> <k>
use std::{collections::BTreeMap, time::Instant};

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::{Index, IndexLabel};

fn sm(i: usize, s: u64) -> u64 {
    let mut h = (i as u64).wrapping_add(s).wrapping_mul(0x9E3779B97F4A7C15);
    h = (h ^ (h >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    h ^= h >> 31;
    h
}
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(200_000);
    let k: usize = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(20_000);
    // Unique timedelta ns labels (distinct strictly-increasing-ish via *1000+i).
    let label_ns: Vec<i64> = (0..n).map(|i| (i as i64) * 1000 + 1).collect();
    let index = Index::new(
        label_ns
            .iter()
            .map(|&v| IndexLabel::Timedelta64(v))
            .collect(),
    );
    let mut cols = BTreeMap::new();
    cols.insert(
        "a".to_string(),
        Column::from_f64_values((0..n).map(|i| sm(i, 7) as f64).collect()),
    );
    let df = DataFrame::new_with_column_order(index, cols, vec!["a".into()]).unwrap();
    // k existing labels chosen pseudo-randomly.
    let sel: Vec<IndexLabel> = (0..k)
        .map(|i| IndexLabel::Timedelta64(label_ns[(sm(i, 3) as usize) % n]))
        .collect();
    let mut best = u128::MAX;
    for _ in 0..6 {
        let t = Instant::now();
        std::hint::black_box(df.loc(&sel).unwrap());
        let e = t.elapsed().as_nanos();
        if e < best {
            best = e;
        }
    }
    println!("td_loc n={n} k={k}: best={best}ns");
}
