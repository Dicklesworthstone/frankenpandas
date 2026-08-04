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
    for (tag, card, stride) in [
        ("bounded-i64", 1000usize, 1i64),
        ("sparse-i64", 200_000usize, 1_000_003i64),
    ] {
        let keys: Vec<Scalar> = (0..n)
            .map(|i| Scalar::Int64(((sm(i, 1) % card as u64) as i64) * stride))
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
        let df = DataFrame::new_with_column_order(idx.clone(), map, vec!["k".into(), "nv".into()])
            .unwrap();
        let k = Series::new("k", idx.clone(), df.column("k").unwrap().clone()).unwrap();
        let sn = Series::new("nv", idx.clone(), df.column("nv").unwrap().clone()).unwrap();
        timeit(&format!("sgb.count {tag} nullable-i64"), || {
            std::hint::black_box(sn.groupby(&k).unwrap().count().unwrap());
        });
    }
}
