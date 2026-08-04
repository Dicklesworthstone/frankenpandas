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
fn build(n: usize, m: usize) -> DataFrame {
    let order: Vec<String> = (0..m).map(|c| format!("c{c}")).collect();
    let mut map = BTreeMap::new();
    for c in 0..m {
        let vv: Vec<Scalar> = (0..n)
            .map(|i| {
                if sm(i, (c as u64) * 97 + 1).is_multiple_of(10) {
                    Scalar::Null(NullKind::NaN)
                } else {
                    Scalar::Float64((sm(i, (c as u64) * 13 + 7) % 100000) as f64)
                }
            })
            .collect();
        map.insert(format!("c{c}"), Column::from_values(vv).unwrap());
    }
    DataFrame::new_with_column_order(Index::from_range(0, n as i64, 1), map, order).unwrap()
}
fn main() {
    for (n, m) in [
        (500_000usize, 8usize),
        (500_000, 12),
        (200_000, 10),
        (200_000, 15),
    ] {
        let df = build(n, m);
        timeit(&format!("corr n={n} m={m}"), || {
            std::hint::black_box(df.corr().unwrap());
        });
    }
}
