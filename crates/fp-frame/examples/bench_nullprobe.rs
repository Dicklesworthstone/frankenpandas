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
    let nv: Vec<Scalar> = (0..n)
        .map(|i| {
            if sm(i, 7).is_multiple_of(5) {
                Scalar::Null(NullKind::Null)
            } else {
                Scalar::Int64((sm(i, 9) % 1000) as i64)
            }
        })
        .collect();
    let idx = Index::from_range(0, n as i64, 1);
    let mut map = BTreeMap::new();
    map.insert("k".into(), Column::from_values(keys).unwrap());
    map.insert("nv".into(), Column::from_values(nv).unwrap());
    let df =
        DataFrame::new_with_column_order(idx.clone(), map, vec!["k".into(), "nv".into()]).unwrap();
    let k = Series::new("k", idx.clone(), df.column("k").unwrap().clone()).unwrap();
    let sn = Series::new("nv", idx.clone(), df.column("nv").unwrap().clone()).unwrap();
    // SeriesGroupBy reductions on nullable Int64 value
    timeit("sgb.sum   nullable-i64", || {
        std::hint::black_box(sn.groupby(&k).unwrap().sum().unwrap());
    });
    timeit("sgb.mean  nullable-i64", || {
        std::hint::black_box(sn.groupby(&k).unwrap().mean().unwrap());
    });
    timeit("sgb.max   nullable-i64", || {
        std::hint::black_box(sn.groupby(&k).unwrap().max().unwrap());
    });
    timeit("sgb.count nullable-i64", || {
        std::hint::black_box(sn.groupby(&k).unwrap().count().unwrap());
    });
    // Series-level ops on nullable Int64
    timeit("s.value_counts nullable-i64", || {
        std::hint::black_box(sn.value_counts().unwrap());
    });
    timeit("s.drop_duplicates nullable-i64", || {
        std::hint::black_box(sn.drop_duplicates().unwrap());
    });
    timeit("s.nunique nullable-i64", || {
        std::hint::black_box(sn.nunique());
    });
    timeit("s.cumsum nullable-i64", || {
        std::hint::black_box(sn.cumsum().unwrap());
    });
    timeit("s.diff   nullable-i64", || {
        std::hint::black_box(sn.diff(1).unwrap());
    });
}
