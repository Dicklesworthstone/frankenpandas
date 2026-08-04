use std::collections::BTreeMap;

use fp_columnar::Column;
use fp_frame::{DataFrame, concat_dataframes};
use fp_index::Index;
use fp_types::Scalar;
fn sm(i: usize, s: u64) -> u64 {
    let mut h = (i as u64).wrapping_add(s).wrapping_mul(0x9E3779B97F4A7C15);
    h = (h ^ (h >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    h ^ (h >> 31)
}
fn mk(n: usize, off: i64) -> DataFrame {
    let idx = Index::from_range(off, off + n as i64, 1);
    let mut m = BTreeMap::new();
    let mut order = vec![];
    for j in 0..5 {
        let nm = format!("c{j}");
        let v: Vec<Scalar> = (0..n)
            .map(|i| {
                if j % 2 == 0 {
                    Scalar::Int64((sm(i, j) % 1000000) as i64)
                } else {
                    Scalar::Float64((sm(i, j) % 100000) as f64 / 100.0)
                }
            })
            .collect();
        m.insert(nm.clone(), Column::from_values(v).unwrap());
        order.push(nm);
    }
    DataFrame::new_with_column_order(idx, m, order).unwrap()
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
    // 10 frames of 200k x 5 = concat to 2M x 5 (few large frames, ignore_index)
    let frames: Vec<DataFrame> = (0..10).map(|k| mk(200_000, k * 200_000)).collect();
    let refs: Vec<&DataFrame> = frames.iter().collect();
    timeit("concat 10x(200kx5) axis0", || {
        std::hint::black_box(concat_dataframes(&refs).unwrap());
    });
    // many small frames: 500 x (4k x 5) = 2M x 5
    let small: Vec<DataFrame> = (0..500).map(|k| mk(4_000, k * 4_000)).collect();
    let srefs: Vec<&DataFrame> = small.iter().collect();
    timeit("concat 500x(4kx5) axis0", || {
        std::hint::black_box(concat_dataframes(&srefs).unwrap());
    });
}
