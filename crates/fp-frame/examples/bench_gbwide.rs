use std::collections::BTreeMap;

use fp_columnar::Column;
use fp_frame::{DataFrame, Series};
use fp_index::Index;
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
fn main() {
    let n = 2_000_000usize;
    let idx = Index::from_range(0, n as i64, 1);
    // WIDE/high-card i64 key (~500k groups)
    let keys: Vec<Scalar> = (0..n)
        .map(|i| Scalar::Int64(((sm(i, 1) % 500_000) as i64) * 7919))
        .collect();
    let vals: Vec<Scalar> = (0..n)
        .map(|i| Scalar::Int64((sm(i, 9) % 1000) as i64))
        .collect();
    let mut map = BTreeMap::new();
    map.insert("k".into(), Column::from_values(keys.clone()).unwrap());
    map.insert("v".into(), Column::from_values(vals.clone()).unwrap());
    let df =
        DataFrame::new_with_column_order(idx.clone(), map, vec!["k".into(), "v".into()]).unwrap();
    let k = Series::new("k", idx.clone(), df.column("k").unwrap().clone()).unwrap();
    let sv = Series::new("v", idx.clone(), df.column("v").unwrap().clone()).unwrap();
    timeit("sgb.sum  wide-i64key", || {
        std::hint::black_box(sv.groupby(&k).unwrap().sum().unwrap());
    });
    timeit("sgb.mean wide-i64key", || {
        std::hint::black_box(sv.groupby(&k).unwrap().mean().unwrap());
    });
    timeit("sgb.max  wide-i64key", || {
        std::hint::black_box(sv.groupby(&k).unwrap().max().unwrap());
    });
    timeit("sgb.count wide-i64key", || {
        std::hint::black_box(sv.groupby(&k).unwrap().count().unwrap());
    });
    timeit("dfgb.sum wide-i64key", || {
        std::hint::black_box(df.groupby(&["k"]).unwrap().sum().unwrap());
    });
    timeit("sgb.transform_sum wide", || {
        std::hint::black_box(sv.groupby(&k).unwrap().transform("sum").unwrap());
    });
}
