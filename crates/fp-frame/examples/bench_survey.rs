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
    timeit("round(2)", || {
        std::hint::black_box(s.round(2).unwrap().len());
    });
    timeit("diff(1)", || {
        std::hint::black_box(s.diff(1).unwrap().len());
    });
    timeit("abs", || {
        std::hint::black_box(s.abs().unwrap().len());
    });
    timeit("clip(0,1)", || {
        std::hint::black_box(s.clip(Some(0.0), Some(1.0)).unwrap().len());
    });
    timeit("fillna(0)", || {
        std::hint::black_box(s.fillna(&Scalar::Float64(0.0)).unwrap().len());
    });
    timeit("dropna(clean)", || {
        std::hint::black_box(s.dropna().unwrap().len());
    });
    let si = Series::new(
        "si",
        Index::from_range(0, n as i64, 1),
        Column::from_i64_values((0..n as i64).collect()),
    )
    .unwrap();
    timeit("fillna_i64(0)", || {
        std::hint::black_box(si.fillna(&Scalar::Int64(0)).unwrap().len());
    });
    timeit("dropna_i64(clean)", || {
        std::hint::black_box(si.dropna().unwrap().len());
    });
    timeit("cumsum", || {
        std::hint::black_box(s.cumsum().unwrap().len());
    });
    timeit("rank(avg)", || {
        std::hint::black_box(s.rank("average", true, "keep").unwrap().len());
    });
    timeit("nlargest100", || {
        std::hint::black_box(s.nlargest(100).unwrap().len());
    });
    timeit("sort_values", || {
        std::hint::black_box(s.sort_values(true).unwrap().len());
    });
}
