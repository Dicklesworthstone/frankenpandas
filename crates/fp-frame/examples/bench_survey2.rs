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
    let f = Column::from_f64_values(
        (0..n as u64).map(|i| {
            let mut z = i.wrapping_mul(0x9E3779B97F4A7C15);
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
            ((z >> 11) as f64) / 1e9
        }).collect(),
    );
    let s = Series::new("s", Index::from_range(0, n as i64, 1), f).unwrap();
    // low-cardinality i64 for value_counts/duplicated/nunique
    let ic = Column::from_i64_values((0..n as i64).map(|i| i % 1000).collect());
    let si = Series::new("si", Index::from_range(0, n as i64, 1), ic).unwrap();
    timeit("sum", || { std::hint::black_box(s.sum().unwrap()); });
    timeit("mean", || { std::hint::black_box(s.mean().unwrap()); });
    timeit("std", || { std::hint::black_box(s.std().unwrap()); });
    timeit("median", || { std::hint::black_box(s.median().unwrap()); });
    timeit("min", || { std::hint::black_box(s.min().unwrap()); });
    timeit("cummax", || { std::hint::black_box(s.cummax().unwrap().len()); });
    timeit("gt_scalar", || { std::hint::black_box(s.gt_scalar(&Scalar::Float64(0.5)).unwrap().len()); });
    let m = s.gt_scalar(&Scalar::Float64(0.5)).unwrap();
    timeit("where(prebuilt)", || { std::hint::black_box(s.where_cond(&m, Some(&Scalar::Float64(0.0))).unwrap().len()); });
    timeit("mask(prebuilt)", || { std::hint::black_box(s.mask(&m, Some(&Scalar::Float64(0.0))).unwrap().len()); });
    timeit("value_counts_i64", || { std::hint::black_box(si.value_counts().unwrap().len()); });
    timeit("duplicated_i64", || { std::hint::black_box(si.duplicated().unwrap().len()); });
    timeit("nunique_i64", || { std::hint::black_box(si.nunique()); });
    timeit("unique_i64", || { std::hint::black_box(si.unique().len()); });
}
