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
fn build(n: usize, utf8key: bool, card: usize) -> DataFrame {
    let keys: Vec<Scalar> = if utf8key {
        (0..n)
            .map(|i| Scalar::Utf8(format!("cat{}", sm(i, 1) % card as u64)))
            .collect()
    } else {
        (0..n)
            .map(|i| Scalar::Int64((sm(i, 1) % card as u64) as i64))
            .collect()
    };
    let nv: Vec<Scalar> = (0..n)
        .map(|i| {
            if sm(i, 7).is_multiple_of(5) {
                Scalar::Null(NullKind::Null)
            } else {
                Scalar::Int64((sm(i, 9) % 1000) as i64)
            }
        })
        .collect();
    let nv2: Vec<Scalar> = (0..n)
        .map(|i| {
            if sm(i, 3).is_multiple_of(5) {
                Scalar::Null(NullKind::Null)
            } else {
                Scalar::Float64((sm(i, 5) % 1000) as f64)
            }
        })
        .collect();
    let idx = Index::from_range(0, n as i64, 1);
    let mut map = BTreeMap::new();
    map.insert("k".into(), Column::from_values(keys).unwrap());
    map.insert("a".into(), Column::from_values(nv).unwrap());
    map.insert("b".into(), Column::from_values(nv2).unwrap());
    DataFrame::new_with_column_order(idx, map, vec!["k".into(), "a".into(), "b".into()]).unwrap()
}
fn main() {
    let n = 2_000_000usize;
    for (tag, utf8, card) in [
        ("bounded-i64", false, 1000usize),
        ("eager-Utf8", true, 1000usize),
    ] {
        let df = build(n, utf8, card);
        timeit(&format!("dfgb.sum   {tag} nullable x2"), || {
            std::hint::black_box(df.groupby(&["k"]).unwrap().sum().unwrap());
        });
        timeit(&format!("dfgb.mean  {tag} nullable x2"), || {
            std::hint::black_box(df.groupby(&["k"]).unwrap().mean().unwrap());
        });
        timeit(&format!("dfgb.count {tag} nullable x2"), || {
            std::hint::black_box(df.groupby(&["k"]).unwrap().count().unwrap());
        });
    }
}
