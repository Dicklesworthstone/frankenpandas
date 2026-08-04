use std::collections::BTreeMap;

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::Index;
use fp_types::{NullKind, Scalar};
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
fn main() {
    let n = 2_000_000usize;
    let k1: Vec<Scalar> = (0..n)
        .map(|i| Scalar::Int64((sm(i, 1) % 50) as i64))
        .collect();
    let k2: Vec<Scalar> = (0..n)
        .map(|i| Scalar::Int64((sm(i, 2) % 40) as i64))
        .collect();
    let a: Vec<Scalar> = (0..n)
        .map(|i| {
            if sm(i, 7).is_multiple_of(5) {
                Scalar::Null(NullKind::Null)
            } else {
                Scalar::Float64((sm(i, 9) % 1000) as f64)
            }
        })
        .collect();
    let b: Vec<Scalar> = (0..n)
        .map(|i| {
            if sm(i, 3).is_multiple_of(5) {
                Scalar::Null(NullKind::Null)
            } else {
                Scalar::Int64((sm(i, 5) % 1000) as i64)
            }
        })
        .collect();
    let idx = Index::from_range(0, n as i64, 1);
    let mut map = BTreeMap::new();
    map.insert("k1".into(), Column::from_values(k1).unwrap());
    map.insert("k2".into(), Column::from_values(k2).unwrap());
    map.insert("a".into(), Column::from_values(a).unwrap());
    map.insert("b".into(), Column::from_values(b).unwrap());
    let df = DataFrame::new_with_column_order(
        idx,
        map,
        vec!["k1".into(), "k2".into(), "a".into(), "b".into()],
    )
    .unwrap();
    timeit("dfgb.sum  multi-i64 nullable x2", || {
        std::hint::black_box(df.groupby(&["k1", "k2"]).unwrap().sum().unwrap());
    });
    timeit("dfgb.mean multi-i64 nullable x2", || {
        std::hint::black_box(df.groupby(&["k1", "k2"]).unwrap().mean().unwrap());
    });
}
