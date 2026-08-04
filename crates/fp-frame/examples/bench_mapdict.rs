use fp_columnar::Column;
use fp_frame::Series;
use fp_index::Index;
use fp_types::Scalar;
fn timeit<F: FnMut()>(label: &str, mut f: F) {
    let mut best = u128::MAX;
    for _ in 0..6 {
        let t = std::time::Instant::now();
        f();
        best = best.min(t.elapsed().as_nanos());
    }
    println!("{label}: best={:.2}ms", best as f64 / 1e6);
}
fn main() {
    let g: Vec<String> = std::env::args().collect();
    let n: usize = g.get(1).and_then(|s| s.parse().ok()).unwrap_or(5_000_000);
    let col = Column::from_i64_values((0..n as i64).map(|i| i % 1000).collect());
    let s = Series::new("s", Index::from_range(0, n as i64, 1), col).unwrap();
    let mapping: Vec<(Scalar, Scalar)> = (0..1000i64)
        .map(|i| (Scalar::Int64(i), Scalar::Int64(i * 2)))
        .collect();
    timeit("map(dict) i64 5M", || {
        std::hint::black_box(s.map(&mapping).unwrap().len());
    });
    timeit("cumsum i64 5M", || {
        std::hint::black_box(s.cumsum().unwrap().len());
    });
    timeit("shift(1) i64 5M", || {
        std::hint::black_box(s.shift_with_fill_value(1, Scalar::Int64(0)).unwrap().len());
    });
    timeit("clip(100,800) i64 5M", || {
        std::hint::black_box(s.clip(Some(100.0), Some(800.0)).unwrap().len());
    });
}
