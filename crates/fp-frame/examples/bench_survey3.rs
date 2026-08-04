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
    println!("{label}: {:.2}ms", best as f64 / 1e6);
}
fn main() {
    let n: usize = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(5_000_000);
    let f = Column::from_f64_values(
        (0..n as u64)
            .map(|i| {
                let mut z = i.wrapping_mul(0x9E3779B97F4A7C15);
                z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
                ((z >> 11) as f64) / 1e9
            })
            .collect(),
    );
    let s = Series::new("s", Index::from_range(0, n as i64, 1), f).unwrap();
    let ic = Column::from_i64_values((0..n as i64).map(|i| i % 1000).collect());
    let si = Series::new("si", Index::from_range(0, n as i64, 1), ic).unwrap();
    let needles: Vec<Scalar> = (0..100i64).map(Scalar::Int64).collect();
    timeit("isin_i64(100)", || {
        std::hint::black_box(si.isin(&needles).unwrap().len());
    });
    timeit("between_f64", || {
        std::hint::black_box(
            s.between(&Scalar::Float64(0.2), &Scalar::Float64(0.8), "both")
                .unwrap()
                .len(),
        );
    });
    timeit("pct_change", || {
        std::hint::black_box(s.pct_change(1).unwrap().len());
    });
    timeit("clip_f64", || {
        std::hint::black_box(s.clip(Some(0.2), Some(0.8)).unwrap().len());
    });
    timeit("cumprod", || {
        std::hint::black_box(s.cumprod().unwrap().len());
    });
    timeit("add_self", || {
        std::hint::black_box(s.add(&s).unwrap().len());
    });
    timeit("gt_series_self", || {
        std::hint::black_box(s.gt(&s).unwrap().len());
    });
}
