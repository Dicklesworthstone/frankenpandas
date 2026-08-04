use std::collections::BTreeMap;

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::Index;
use fp_types::Scalar;
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
fn mkdf(n: usize, off: i64) -> DataFrame {
    let idx = Index::from_range(off, off + n as i64, 1);
    let mut m = BTreeMap::new();
    let mut order = vec![];
    for c in 0..5 {
        let nm = format!("c{c}");
        let col: Vec<Scalar> = (0..n)
            .map(|i| Scalar::Float64((sm(i, c as u64 + 1) % 1000) as f64))
            .collect();
        m.insert(nm.clone(), Column::from_values(col).unwrap());
        order.push(nm);
    }
    DataFrame::new_with_column_order(idx, m, order).unwrap()
}
fn main() {
    let n = 1_000_000usize;
    let a = mkdf(n, 0);
    let bs = mkdf(n, 500_000);
    timeit("df.gt unaligned (5col,1M)", || {
        std::hint::black_box(a.gt(&bs).unwrap());
    });
    timeit("df.eq unaligned (5col,1M)", || {
        std::hint::black_box(a.eq(&bs).unwrap());
    });
}
