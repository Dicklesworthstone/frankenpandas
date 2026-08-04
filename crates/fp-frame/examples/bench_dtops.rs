use fp_columnar::Column;
use fp_frame::Series;
use fp_index::Index;
use fp_types::Scalar;
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
fn main() {
    let n = 2_000_000usize;
    let base = 1_600_000_000_000_000_000i64; // ns
    let dv: Vec<Scalar> = (0..n)
        .map(|i| {
            Scalar::Datetime64(
                base + (sm(i, 1) % (4 * 365 * 24 * 3600) as u64) as i64 * 1_000_000_000,
            )
        })
        .collect();
    let idx = Index::from_range(0, n as i64, 1);
    let s = Series::new("s", idx, Column::from_values(dv).unwrap()).unwrap();
    timeit("dt.year", || {
        std::hint::black_box(s.dt().year().unwrap());
    });
    timeit("dt.month", || {
        std::hint::black_box(s.dt().month().unwrap());
    });
    timeit("dt.day", || {
        std::hint::black_box(s.dt().day().unwrap());
    });
    timeit("dt.dayofweek", || {
        std::hint::black_box(s.dt().dayofweek().unwrap());
    });
    timeit("dt.floor(D)", || {
        std::hint::black_box(s.dt().floor("D").unwrap());
    });
    timeit("dt.hour", || {
        std::hint::black_box(s.dt().hour().unwrap());
    });
}
