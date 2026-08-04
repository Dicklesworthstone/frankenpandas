//! df.pivot_table(values,index,columns,aggfunc) @1M. Run: bench_pivottable <n> <agg>
use std::collections::BTreeMap;

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
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(1_000_000);
    let agg = a.get(2).map(String::as_str).unwrap_or("mean");
    let (icard, ccard) = (1000u64, 50u64);
    let mut cols = BTreeMap::new();
    cols.insert(
        "i".to_string(),
        Column::from_i64_values((0..n).map(|x| (sm(x, 0) % icard) as i64).collect()),
    );
    cols.insert(
        "c".to_string(),
        Column::from_i64_values((0..n).map(|x| (sm(x, 1) % ccard) as i64).collect()),
    );
    cols.insert(
        "v".to_string(),
        Column::from_f64_values((0..n).map(|x| sm(x, 2) as f64).collect()),
    );
    let labels: Vec<IndexLabel> = (0..n as i64).map(IndexLabel::Int64).collect();
    let df = DataFrame::new_with_column_order(
        Index::new(labels),
        cols,
        vec!["i".into(), "c".into(), "v".into()],
    )
    .unwrap();
    let mut best = u128::MAX;
    for _ in 0..6 {
        let t = std::time::Instant::now();
        std::hint::black_box(df.pivot_table("v", "i", "c", agg).unwrap());
        let e = t.elapsed().as_nanos();
        if e < best {
            best = e;
        }
    }
    println!("pivottable_{agg} n={n}: best={best}ns");
}
