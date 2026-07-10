use std::collections::BTreeMap;

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::Index;
use fp_join::{AsofDirection, merge_asof};
use fp_types::Scalar;
fn mk(n: usize, mul: i64) -> DataFrame {
    let idx = Index::from_range(0, n as i64, 1);
    let mut m = BTreeMap::new();
    let base = 1_600_000_000_000_000_000i64;
    m.insert(
        "t".to_string(),
        Column::from_values(
            (0..n)
                .map(|i| Scalar::Datetime64(base + i as i64 * mul))
                .collect(),
        )
        .unwrap(),
    );
    m.insert(
        "v".to_string(),
        Column::from_values((0..n).map(|i| Scalar::Float64(i as f64)).collect()).unwrap(),
    );
    DataFrame::new_with_column_order(idx, m, vec!["t".to_string(), "v".to_string()]).unwrap()
}
fn main() {
    let n = 1_000_000usize;
    let l = mk(n, 3000);
    let rt = mk(n, 2000);
    let mut best = u128::MAX;
    for _ in 0..8 {
        let t = std::time::Instant::now();
        std::hint::black_box(merge_asof(&l, &rt, "t", AsofDirection::Backward).unwrap());
        best = best.min(t.elapsed().as_nanos());
    }
    println!("asof datetime 1M/1M: {:.2}ms", best as f64 / 1e6);
}
