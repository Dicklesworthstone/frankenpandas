//! DataFrame.sort_values by a Float64 column. Run: -- 2000000 <card?>
use std::{collections::BTreeMap, time::Instant};

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::{Index, IndexLabel};
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(2_000_000);
    let it: usize = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(8);
    let card: i64 = a.get(3).and_then(|s| s.parse().ok()).unwrap_or(i64::MAX);
    let index = Index::new((0..n as i64).map(IndexLabel::Int64).collect());
    let mut cols = BTreeMap::new();
    cols.insert(
        "k".to_string(),
        Column::from_f64_values(
            (0..n)
                .map(|i| {
                    let mut h = (i as u64).wrapping_mul(0x9E3779B97F4A7C15);
                    h = (h ^ (h >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
                    h ^= h >> 31;
                    ((h >> 1) as i64).rem_euclid(card) as f64
                })
                .collect(),
        ),
    );
    cols.insert(
        "v".to_string(),
        Column::from_f64_values((0..n).map(|i| i as f64).collect()),
    );
    let df = DataFrame::new_with_column_order(index, cols, vec!["k".into(), "v".into()]).unwrap();
    let mut best = u128::MAX;
    for _ in 0..it {
        let t = Instant::now();
        std::hint::black_box(df.sort_values("k", true).unwrap());
        let e = t.elapsed().as_nanos();
        if e < best {
            best = e;
        }
    }
    println!("df_sortvals n={n} card={card}: best={best}ns");
}
