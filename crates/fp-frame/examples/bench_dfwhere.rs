use std::collections::BTreeMap;

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::Index;
use fp_types::Scalar;
fn main() {
    let n: usize = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(2_000_000);
    let ncols = 5;
    let idx = Index::from_range(0, n as i64, 1);
    let mut cols = BTreeMap::new();
    let mut conds = BTreeMap::new();
    for c in 0..ncols {
        let name = format!("c{c}");
        cols.insert(
            name.clone(),
            Column::from_f64_values((0..n).map(|i| (i as f64) * 0.5 + c as f64).collect()),
        );
        conds.insert(
            name,
            Column::from_bool_values((0..n).map(|i| (i + c) % 2 == 0).collect()),
        );
    }
    let df = DataFrame::new(idx.clone(), cols).unwrap();
    let cond = DataFrame::new(idx, conds).unwrap();
    let mut best = u128::MAX;
    for _ in 0..6 {
        let t = std::time::Instant::now();
        let r = df.where_cond(&cond, Some(&Scalar::Float64(0.0))).unwrap();
        std::hint::black_box(r.shape());
        best = best.min(t.elapsed().as_nanos());
    }
    println!(
        "df.where f64 {}x{} : best={:.2}ms",
        n,
        ncols,
        best as f64 / 1e6
    );
}
