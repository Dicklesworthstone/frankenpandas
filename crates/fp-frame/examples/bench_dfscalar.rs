//! DataFrame.add_scalar on a nullable f64 column. bench_dfscalar <n>
use std::collections::BTreeMap;

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::{Index, IndexLabel};
fn main() {
    let n: usize = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(5_000_000);
    let mut cols = BTreeMap::new();
    cols.insert(
        "a".to_string(),
        Column::from_f64_values(
            (0..n as u64)
                .map(|i| {
                    let mut z = i.wrapping_mul(0x9E3779B97F4A7C15);
                    z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
                    if z % 10 == 0 {
                        f64::NAN
                    } else {
                        (z >> 11) as f64 / 1e9
                    }
                })
                .collect(),
        ),
    );
    let df = DataFrame::new_with_column_order(
        Index::new((0..n as i64).map(IndexLabel::Int64).collect()),
        cols,
        vec!["a".into()],
    )
    .unwrap();
    let mut best = u128::MAX;
    for _ in 0..6 {
        let t = std::time::Instant::now();
        std::hint::black_box(df.add_scalar(1.0).unwrap().shape());
        best = best.min(t.elapsed().as_nanos());
    }
    println!("df.add_scalar nullable n={n}: {:.2}ms", best as f64 / 1e6);
}
