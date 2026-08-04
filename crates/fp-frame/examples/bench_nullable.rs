//! Series ops on a NULLABLE f64 column (~10% NaN). bench_nullable <n>
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
    // ~10% NaN
    let f = Column::from_f64_values(
        (0..n as u64)
            .map(|i| {
                let mut z = i.wrapping_mul(0x9E3779B97F4A7C15);
                z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
                if z % 10 == 0 {
                    f64::NAN
                } else {
                    ((z >> 11) as f64) / 1e9
                }
            })
            .collect(),
    );
    let s = Series::new("s", Index::from_range(0, n as i64, 1), f).unwrap();
    timeit("sum", || {
        std::hint::black_box(s.sum().unwrap());
    });
    timeit("mean", || {
        std::hint::black_box(s.mean().unwrap());
    });
    timeit("abs", || {
        std::hint::black_box(s.abs().unwrap().len());
    });
    timeit("cumsum", || {
        std::hint::black_box(s.cumsum().unwrap().len());
    });
    timeit("round", || {
        std::hint::black_box(s.round(2).unwrap().len());
    });
    timeit("clip", || {
        std::hint::black_box(s.clip(Some(0.1), Some(0.9)).unwrap().len());
    });
    timeit("fillna0", || {
        std::hint::black_box(s.fillna(&Scalar::Float64(0.0)).unwrap().len());
    });
    timeit("gt_scalar", || {
        std::hint::black_box(s.gt_scalar(&Scalar::Float64(0.5)).unwrap().len());
    });
    timeit("neg", || {
        std::hint::black_box(s.neg().unwrap().len());
    });
    timeit("sqrt", || {
        std::hint::black_box(s.sqrt().unwrap().len());
    });
    timeit("exp", || {
        std::hint::black_box(s.exp().unwrap().len());
    });
    timeit("diff", || {
        std::hint::black_box(s.diff(1).unwrap().len());
    });
    timeit("cummax", || {
        std::hint::black_box(s.cummax().unwrap().len());
    });
    timeit("cummin", || {
        std::hint::black_box(s.cummin().unwrap().len());
    });
    let o = s.clone();
    timeit("add_col", || {
        std::hint::black_box(s.add(&o).unwrap().len());
    });
    timeit("mul_col", || {
        std::hint::black_box(s.mul(&o).unwrap().len());
    });
    timeit("ffill", || {
        std::hint::black_box(s.ffill(None).unwrap().len());
    });
    timeit("bfill", || {
        std::hint::black_box(s.bfill(None).unwrap().len());
    });
    timeit("interpolate", || {
        std::hint::black_box(s.interpolate().unwrap().len());
    });
    timeit("between", || {
        std::hint::black_box(
            s.between(&Scalar::Float64(0.1), &Scalar::Float64(0.9), "both")
                .unwrap()
                .len(),
        );
    });
    let needles: Vec<Scalar> = (0..1000)
        .map(|i| Scalar::Float64(i as f64 * 0.001))
        .collect();
    timeit("isin", || {
        std::hint::black_box(s.isin(&needles).unwrap().len());
    });
    let lo = Series::new(
        "lo",
        Index::from_range(0, n as i64, 1),
        Column::from_f64_values((0..n).map(|i| (i % 100) as f64 * 0.001).collect()),
    )
    .unwrap();
    let hi = Series::new(
        "hi",
        Index::from_range(0, n as i64, 1),
        Column::from_f64_values((0..n).map(|i| 0.5 + (i % 100) as f64 * 0.001).collect()),
    )
    .unwrap();
    timeit("clip_series", || {
        std::hint::black_box(s.clip_with_series(Some(&lo), Some(&hi)).unwrap().len());
    });
    let mask = Series::new(
        "m",
        Index::from_range(0, n as i64, 1),
        Column::from_bool_values((0..n).map(|i| i % 2 == 0).collect()),
    )
    .unwrap();
    timeit("where", || {
        std::hint::black_box(
            s.where_cond(&mask, Some(&Scalar::Float64(0.0)))
                .unwrap()
                .len(),
        );
    });
    timeit("mask", || {
        std::hint::black_box(s.mask(&mask, Some(&Scalar::Float64(0.0))).unwrap().len());
    });
    timeit("where_series", || {
        std::hint::black_box(s.where_cond_series(&mask, &o).unwrap().len());
    });
}
