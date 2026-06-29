//! Series ops on a NULLABLE f64 column (~10% NaN). bench_nullable <n>
use fp_frame::Series;
use fp_columnar::Column;
use fp_index::Index;
use fp_types::Scalar;
fn timeit<F: FnMut()>(label: &str, mut f: F) {
    let mut best = u128::MAX;
    for _ in 0..6 {
        let t = std::time::Instant::now();
        f();
        best = best.min(t.elapsed().as_nanos());
    }
    println!("{label}: {:.2}ms", best as f64 / 1e6);
}
fn main() {
    let n: usize = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(5_000_000);
    // ~10% NaN
    let f = Column::from_f64_values((0..n as u64).map(|i| {
        let mut z = i.wrapping_mul(0x9E3779B97F4A7C15); z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        if z % 10 == 0 { f64::NAN } else { ((z >> 11) as f64) / 1e9 }
    }).collect());
    let s = Series::new("s", Index::from_range(0, n as i64, 1), f).unwrap();
    timeit("sum", || { std::hint::black_box(s.sum().unwrap()); });
    timeit("mean", || { std::hint::black_box(s.mean().unwrap()); });
    timeit("abs", || { std::hint::black_box(s.abs().unwrap().len()); });
    timeit("cumsum", || { std::hint::black_box(s.cumsum().unwrap().len()); });
    timeit("round", || { std::hint::black_box(s.round(2).unwrap().len()); });
    timeit("clip", || { std::hint::black_box(s.clip(Some(0.1), Some(0.9)).unwrap().len()); });
    timeit("fillna0", || { std::hint::black_box(s.fillna(&Scalar::Float64(0.0)).unwrap().len()); });
    timeit("gt_scalar", || { std::hint::black_box(s.gt_scalar(&Scalar::Float64(0.5)).unwrap().len()); });
}
