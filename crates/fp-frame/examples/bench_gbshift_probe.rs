use std::collections::BTreeMap;

use fp_columnar::Column;
use fp_frame::{DataFrame, Series};
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
    let card = 1000usize;
    let keys: Vec<Scalar> = (0..n)
        .map(|i| Scalar::Int64((sm(i, 1) % card as u64) as i64))
        .collect();
    let iv: Vec<Scalar> = (0..n)
        .map(|i| {
            if sm(i, 7).is_multiple_of(5) {
                Scalar::Null(NullKind::Null)
            } else {
                Scalar::Int64((sm(i, 9) % 1000) as i64)
            }
        })
        .collect();
    let sv: Vec<Scalar> = (0..n)
        .map(|i| {
            if sm(i, 11).is_multiple_of(5) {
                Scalar::Null(NullKind::Null)
            } else {
                Scalar::Utf8(format!("v{}", sm(i, 13) % 200))
            }
        })
        .collect();
    let idx = Index::from_range(0, n as i64, 1);
    let mut map = BTreeMap::new();
    map.insert("k".to_string(), Column::from_values(keys).unwrap());
    map.insert("iv".to_string(), Column::from_values(iv).unwrap());
    map.insert("sv".to_string(), Column::from_values(sv).unwrap());
    let df = DataFrame::new_with_column_order(
        idx.clone(),
        map,
        vec!["k".into(), "iv".into(), "sv".into()],
    )
    .unwrap();
    let k = Series::new("k", idx.clone(), df.column("k").unwrap().clone()).unwrap();
    let si = Series::new("iv", idx.clone(), df.column("iv").unwrap().clone()).unwrap();
    let ss = Series::new("sv", idx.clone(), df.column("sv").unwrap().clone()).unwrap();
    timeit("gb.shift(1)  Int64", || {
        std::hint::black_box(si.groupby(&k).unwrap().shift(1).unwrap());
    });
    timeit("gb.shift(1)  Utf8 ", || {
        std::hint::black_box(ss.groupby(&k).unwrap().shift(1).unwrap());
    });
    timeit("gb.shift(-2) Utf8 ", || {
        std::hint::black_box(ss.groupby(&k).unwrap().shift(-2).unwrap());
    });
    timeit("gb.diff(1)   Int64", || {
        std::hint::black_box(si.groupby(&k).unwrap().diff(1).unwrap());
    });
}
