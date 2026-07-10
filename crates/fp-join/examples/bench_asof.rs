use std::collections::BTreeMap;

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::Index;
use fp_join::{AsofDirection, merge_asof};
use fp_types::Scalar;
fn sm(i: usize, s: u64) -> u64 {
    let mut h = (i as u64).wrapping_add(s).wrapping_mul(0x9E3779B97F4A7C15);
    h = (h ^ (h >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    h ^ (h >> 31)
}
fn timeit(l: &str, mut f: impl FnMut()) {
    let mut b = u128::MAX;
    for _ in 0..6 {
        let t = std::time::Instant::now();
        f();
        b = b.min(t.elapsed().as_nanos());
    }
    println!("{l}: {:.2}ms", b as f64 / 1e6);
}
fn mk(n: usize, keymul: i64, val_seed: u64) -> DataFrame {
    let idx = Index::from_range(0, n as i64, 1);
    let mut m = BTreeMap::new();
    let mut order = vec![];
    // sorted key: i*keymul (non-decreasing)
    let k: Vec<Scalar> = (0..n).map(|i| Scalar::Int64(i as i64 * keymul)).collect();
    let v: Vec<Scalar> = (0..n)
        .map(|i| Scalar::Float64((sm(i, val_seed) % 1000) as f64))
        .collect();
    m.insert("t".to_string(), Column::from_values(k).unwrap());
    order.push("t".to_string());
    m.insert("v".to_string(), Column::from_values(v).unwrap());
    order.push("v".to_string());
    DataFrame::new_with_column_order(idx, m, order).unwrap()
}
fn main() {
    let n = 1_000_000usize;
    let left = mk(n, 3, 1); // keys 0,3,6,...
    let right = mk(n, 2, 2); // keys 0,2,4,... (denser, backward matches)
    timeit("merge_asof backward 1M/1M", || {
        std::hint::black_box(merge_asof(&left, &right, "t", AsofDirection::Backward).unwrap());
    });
}
