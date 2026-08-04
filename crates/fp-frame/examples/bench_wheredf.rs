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
fn mkf(off: i64, n: usize, ncol: usize, nullable: bool) -> DataFrame {
    let idx = Index::from_range(off, off + n as i64, 1);
    let mut m = BTreeMap::new();
    let mut order = vec![];
    for c in 0..ncol {
        let nm = format!("c{c}");
        let v: Vec<Scalar> = (0..n)
            .map(|i| {
                if nullable && sm(i, c as u64 + 7).is_multiple_of(5) {
                    Scalar::Null(NullKind::Null)
                } else {
                    Scalar::Float64((sm(i, c as u64 + 1) % 100) as f64)
                }
            })
            .collect();
        m.insert(nm.clone(), Column::from_values(v).unwrap());
        order.push(nm);
    }
    DataFrame::new_with_column_order(idx, m, order).unwrap()
}
fn mkbool(off: i64, n: usize, ncol: usize) -> DataFrame {
    let idx = Index::from_range(off, off + n as i64, 1);
    let mut m = BTreeMap::new();
    let mut order = vec![];
    for c in 0..ncol {
        let nm = format!("c{c}");
        let v: Vec<Scalar> = (0..n)
            .map(|i| Scalar::Bool(sm(i, c as u64 + 3).is_multiple_of(2)))
            .collect();
        m.insert(nm.clone(), Column::from_values(v).unwrap());
        order.push(nm);
    }
    DataFrame::new_with_column_order(idx, m, order).unwrap()
}
fn main() {
    let n = 1_000_000usize;
    let ncol = 5;
    let s = mkf(0, n, ncol, true);
    let cond = mkbool(200_000, n, ncol); // shifted index -> unaligned
    let other = mkf(200_000, n, ncol, true);
    timeit("df.where UNALIGNED nullable (1M x5)", || {
        std::hint::black_box(s.where_cond_df(&cond, &other).unwrap());
    });
    let s2 = mkf(0, n, ncol, false);
    let other2 = mkf(200_000, n, ncol, false);
    timeit("df.where UNALIGNED all-valid (1M x5)", || {
        std::hint::black_box(s2.where_cond_df(&cond, &other2).unwrap());
    });
}
