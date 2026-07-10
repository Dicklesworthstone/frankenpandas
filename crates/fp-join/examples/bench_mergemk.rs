use std::collections::BTreeMap;

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::Index;
use fp_join::{JoinType, merge_dataframes_on};
use fp_types::Scalar;
fn sm(i: usize, s: u64) -> u64 {
    let mut h = (i as u64).wrapping_add(s).wrapping_mul(0x9E3779B97F4A7C15);
    h = (h ^ (h >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    h ^ (h >> 31)
}
fn timeit(l: &str, mut f: impl FnMut()) {
    let mut b = u128::MAX;
    for _ in 0..5 {
        let t = std::time::Instant::now();
        f();
        b = b.min(t.elapsed().as_nanos());
    }
    println!("{l}: {:.2}ms", b as f64 / 1e6);
}
fn mkdf(n: usize, seed: u64, vn: &str) -> DataFrame {
    let idx = Index::from_range(0, n as i64, 1);
    let k1: Vec<Scalar> = (0..n)
        .map(|i| Scalar::Int64(((sm(i, seed) % (n as u64)) as i64) * 2654435761))
        .collect();
    let k2: Vec<Scalar> = (0..n)
        .map(|i| Scalar::Int64((sm(i, seed + 10) % 1000) as i64))
        .collect();
    let v: Vec<Scalar> = (0..n)
        .map(|i| Scalar::Float64((sm(i, 7) % 1000) as f64))
        .collect();
    let mut m = BTreeMap::new();
    m.insert("k1".into(), Column::from_values(k1).unwrap());
    m.insert("k2".into(), Column::from_values(k2).unwrap());
    m.insert(vn.into(), Column::from_values(v).unwrap());
    DataFrame::new_with_column_order(idx, m, vec!["k1".into(), "k2".into(), vn.into()]).unwrap()
}
fn main() {
    let n = 1_000_000usize;
    let l = mkdf(n, 1, "lval");
    let r = mkdf(n, 2, "rval");
    let on = &["k1", "k2"];
    timeit("merge2 inner wide-i64x2", || {
        std::hint::black_box(merge_dataframes_on(&l, &r, on, JoinType::Inner).unwrap());
    });
    timeit("merge2 left wide-i64x2", || {
        std::hint::black_box(merge_dataframes_on(&l, &r, on, JoinType::Left).unwrap());
    });
    timeit("merge2 outer wide-i64x2", || {
        std::hint::black_box(merge_dataframes_on(&l, &r, on, JoinType::Outer).unwrap());
    });
}
