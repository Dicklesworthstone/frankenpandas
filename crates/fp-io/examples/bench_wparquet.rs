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
fn main() {
    let n = 1_000_000usize;
    let idx = Index::from_range(0, n as i64, 1);
    let mut m = BTreeMap::new();
    let mut order = vec![];
    for j in 0..6 {
        let nm = format!("c{j}");
        let v: Vec<Scalar> = (0..n)
            .map(|i| {
                if j % 2 == 0 {
                    Scalar::Int64((sm(i, j) % 1_000_000) as i64)
                } else {
                    Scalar::Float64((sm(i, j) % 100000) as f64 / 100.0)
                }
            })
            .collect();
        m.insert(nm.clone(), Column::from_values(v).unwrap());
        order.push(nm);
    }
    let df = DataFrame::new_with_column_order(idx, m, order).unwrap();
    let mut best = u128::MAX;
    for _ in 0..6 {
        let t = std::time::Instant::now();
        let bytes = fp_io::write_parquet_bytes(&df).unwrap();
        std::hint::black_box(bytes.len());
        best = best.min(t.elapsed().as_nanos());
    }
    println!("fp write_parquet 1Mx6 numeric: {:.2}ms", best as f64 / 1e6);
}
