use std::collections::BTreeMap;

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::Index;
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
    let n = 1_000_000usize;
    let idx = Index::from_range(0, n as i64, 1);
    let mut m = BTreeMap::new();
    let mut order = vec![];
    for j in 0..5 {
        let nm = format!("c{j}");
        m.insert(
            nm.clone(),
            Column::from_f64_values((0..n).map(|i| (sm(i, j as u64) % 10000) as f64).collect()),
        );
        order.push(nm);
    }
    let df = DataFrame::new_with_column_order(idx, m, order).unwrap();
    timeit("df.diff(1) 1Mx5", || {
        std::hint::black_box(df.diff(1).unwrap());
    });
}
