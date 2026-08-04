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
    for _ in 0..5 {
        let t = std::time::Instant::now();
        f();
        b = b.min(t.elapsed().as_nanos());
    }
    println!("{l}: {:.2}ms", b as f64 / 1e6);
}
fn main() {
    let n = 1_000_000usize;
    let idx = Index::from_range(0, n as i64, 1);
    // df with 20 nullable-f64 columns for axis-0 reductions
    let mut map = BTreeMap::new();
    let mut order = vec![];
    for c in 0..20 {
        let nm = format!("c{c}");
        let col: Vec<Scalar> = (0..n)
            .map(|i| {
                if sm(i, c as u64 + 1).is_multiple_of(7) {
                    Scalar::Null(NullKind::Null)
                } else {
                    Scalar::Float64((sm(i, c as u64 + 9) % 1000) as f64)
                }
            })
            .collect();
        map.insert(nm.clone(), Column::from_values(col).unwrap());
        order.push(nm);
    }
    let df = DataFrame::new_with_column_order(idx.clone(), map, order).unwrap();
    timeit("df.sum(axis=0) 20 nullable-f64", || {
        std::hint::black_box(df.sum().unwrap());
    });
    timeit("df.mean(axis=0) 20 nullable-f64", || {
        std::hint::black_box(df.mean().unwrap());
    });
    timeit("df.std(axis=0) 20 nullable-f64", || {
        std::hint::black_box(df.std().unwrap());
    });
    // Series nullable ops
    let sv: Vec<Scalar> = (0..n)
        .map(|i| {
            if sm(i, 7).is_multiple_of(6) {
                Scalar::Null(NullKind::Null)
            } else {
                Scalar::Int64((sm(i, 9) % 5000) as i64)
            }
        })
        .collect();
    let s = Series::new("s", idx.clone(), Column::from_values(sv).unwrap()).unwrap();
    timeit("Series.value_counts nullable-i64", || {
        std::hint::black_box(s.value_counts().unwrap());
    });
    timeit("Series.rank nullable-i64", || {
        std::hint::black_box(s.rank("average", true, "keep").unwrap());
    });
}
