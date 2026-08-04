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

fn build(n: usize, card: usize) -> DataFrame {
    // dense i64 group key in [0,card)
    let keys: Vec<Scalar> = (0..n)
        .map(|i| Scalar::Int64((sm(i, 1) % card as u64) as i64))
        .collect();
    // nullable Int64 value: 20% missing
    let iv: Vec<Scalar> = (0..n)
        .map(|i| {
            if sm(i, 7).is_multiple_of(5) {
                Scalar::Null(NullKind::Null)
            } else {
                Scalar::Int64((sm(i, 9) % 1000) as i64)
            }
        })
        .collect();
    // Utf8 value: 20% missing, ~200 distinct
    let sv: Vec<Scalar> = (0..n)
        .map(|i| {
            if sm(i, 11).is_multiple_of(5) {
                Scalar::Null(NullKind::Null)
            } else {
                Scalar::Utf8(format!("v{}", sm(i, 13) % 200))
            }
        })
        .collect();
    let mut map = BTreeMap::new();
    map.insert("k".to_string(), Column::from_values(keys).unwrap());
    map.insert("iv".to_string(), Column::from_values(iv).unwrap());
    map.insert("sv".to_string(), Column::from_values(sv).unwrap());
    DataFrame::new_with_column_order(
        Index::from_range(0, n as i64, 1),
        map,
        vec!["k".into(), "iv".into(), "sv".into()],
    )
    .unwrap()
}

fn main() {
    let n = 2_000_000usize;
    {
        let card = 1000usize;
        let df = build(n, card);
        let sv_i = df.column("iv").unwrap().clone();
        let sv_s = df.column("sv").unwrap().clone();
        let k = df.column("k").unwrap().clone();
        let idx = df.index().clone();
        let si = fp_frame::Series::new("iv", idx.clone(), sv_i).unwrap();
        let ss = fp_frame::Series::new("sv", idx.clone(), sv_s).unwrap();
        let ki = fp_frame::Series::new("k", idx.clone(), k.clone()).unwrap();
        timeit(&format!("gb.ffill  Int64 n={n} card={card}"), || {
            std::hint::black_box(si.groupby(&ki).unwrap().ffill(None).unwrap());
        });
        timeit(&format!("gb.ffill  Utf8  n={n} card={card}"), || {
            std::hint::black_box(ss.groupby(&ki).unwrap().ffill(None).unwrap());
        });
        timeit(&format!("gb.fillna Int64 n={n} card={card}"), || {
            std::hint::black_box(si.groupby(&ki).unwrap().fillna(&Scalar::Int64(0)).unwrap());
        });
        timeit(&format!("gb.fillna Utf8  n={n} card={card}"), || {
            std::hint::black_box(
                ss.groupby(&ki)
                    .unwrap()
                    .fillna(&Scalar::Utf8("X".into()))
                    .unwrap(),
            );
        });
    }
}
