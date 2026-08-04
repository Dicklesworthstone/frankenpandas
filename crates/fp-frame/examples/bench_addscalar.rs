//! DataFrame::add_scalar over a 5M single-column frame. bench_addscalar <n>
use std::collections::BTreeMap;

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::Index;
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(5_000_000);
    let col = Column::from_f64_values((0..n).map(|i| (i % 1000) as f64 * 0.5).collect());
    let mut cols = BTreeMap::new();
    cols.insert("a".to_string(), col);
    let df = DataFrame::new(Index::from_range(0, n as i64, 1), cols).unwrap();
    let mut best = u128::MAX;
    for _ in 0..6 {
        let t = std::time::Instant::now();
        let r = df.add_scalar(5.0).unwrap();
        std::hint::black_box(r.shape());
        best = best.min(t.elapsed().as_nanos());
    }
    println!(
        "add_scalar f64 n={n}: best={best}ns ({:.2}ms)",
        best as f64 / 1e6
    );
}
