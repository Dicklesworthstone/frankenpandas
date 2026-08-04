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
    for _ in 0..5 {
        let t = std::time::Instant::now();
        f();
        b = b.min(t.elapsed().as_nanos());
    }
    println!("{l}: {:.2}ms", b as f64 / 1e6);
}
fn main() {
    let n = 1_000_000usize;
    let idxk: Vec<Scalar> = (0..n)
        .map(|i| Scalar::Int64((sm(i, 1) % 200) as i64))
        .collect();
    let colk: Vec<Scalar> = (0..n)
        .map(|i| Scalar::Int64((sm(i, 2) % 50) as i64))
        .collect();
    let val: Vec<Scalar> = (0..n)
        .map(|i| {
            if sm(i, 7).is_multiple_of(5) {
                Scalar::Null(NullKind::Null)
            } else {
                Scalar::Float64((sm(i, 9) % 1000) as f64)
            }
        })
        .collect();
    let idx = Index::from_range(0, n as i64, 1);
    let mut map = BTreeMap::new();
    map.insert("r".into(), Column::from_values(idxk).unwrap());
    map.insert("c".into(), Column::from_values(colk).unwrap());
    map.insert("v".into(), Column::from_values(val).unwrap());
    let df = DataFrame::new_with_column_order(idx, map, vec!["r".into(), "c".into(), "v".into()])
        .unwrap();
    for agg in ["sum", "mean"] {
        timeit(&format!("pivot_table {agg} nullable"), || {
            std::hint::black_box(df.pivot_table("v", "r", "c", agg).unwrap());
        });
    }
}
