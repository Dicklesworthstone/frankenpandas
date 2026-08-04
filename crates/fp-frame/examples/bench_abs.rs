use fp_columnar::Column;
use fp_frame::Series;
use fp_index::Index;
use fp_types::Scalar;
fn timeit(l: &str, mut f: impl FnMut()) {
    let mut b = u128::MAX;
    for _ in 0..8 {
        let t = std::time::Instant::now();
        f();
        b = b.min(t.elapsed().as_nanos());
    }
    println!("{l}: {:.2}ms", b as f64 / 1e6);
}
fn main() {
    let n = 5_000_000usize;
    let idx = Index::from_range(0, n as i64, 1);
    let v: Vec<Scalar> = (0..n)
        .map(|i| Scalar::Float64((i as f64) * 0.5 - 1000.0))
        .collect();
    let s = Series::new("s", idx, Column::from_values(v).unwrap()).unwrap();
    timeit("s.abs 5M", || {
        std::hint::black_box(s.abs().unwrap());
    });
    timeit("s.round(2) 5M", || {
        std::hint::black_box(s.round(2).unwrap());
    });
    timeit("s.clip(-500,500) 5M", || {
        std::hint::black_box(s.clip(Some(-500.0), Some(500.0)).unwrap());
    });
}
