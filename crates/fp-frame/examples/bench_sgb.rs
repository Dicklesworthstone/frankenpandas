//! SeriesGroupBy by a Utf8 key .sum(). Run: -- 1000000 1000 20
use std::time::Instant;

use fp_frame::Series;
use fp_index::IndexLabel;
use fp_types::Scalar;
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(1_000_000);
    let card: i64 = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(1000);
    let iters: usize = a.get(3).and_then(|s| s.parse().ok()).unwrap_or(20);
    let labels: Vec<IndexLabel> = (0..n as i64).map(IndexLabel::Int64).collect();
    let by = Series::from_values(
        "k",
        labels.clone(),
        (0..n)
            .map(|i| {
                Scalar::Utf8(format!(
                    "k{:04}",
                    ((i as i64).wrapping_mul(2654435761) >> 13) % card
                ))
            })
            .collect(),
    )
    .unwrap();
    let v = Series::from_values("v", labels, (0..n as i64).map(Scalar::Int64).collect()).unwrap();
    let mut best = u128::MAX;
    for _ in 0..iters {
        let t = Instant::now();
        std::hint::black_box(v.groupby(&by).unwrap().sum().unwrap());
        let e = t.elapsed().as_nanos();
        if e < best {
            best = e;
        }
    }
    println!("series_groupby_utf8 n={n} card={card}: best={best}ns");
}
