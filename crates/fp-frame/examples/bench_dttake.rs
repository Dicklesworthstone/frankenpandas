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
    for _ in 0..8 {
        let t = std::time::Instant::now();
        f();
        b = b.min(t.elapsed().as_nanos());
    }
    println!("{l}: {:.2}ms", b as f64 / 1e6);
}
fn main() {
    let n = 2_000_000usize;
    let idx = Index::from_range(0, n as i64, 1);
    let base = 1_600_000_000_000_000_000i64;
    let dt = Series::new(
        "dt",
        idx.clone(),
        Column::from_datetime64_values((0..n).map(|i| base + i as i64 * 1_000_000_000).collect()),
    )
    .unwrap();
    let i64s = Series::new(
        "i",
        idx,
        Column::from_values((0..n).map(|i| Scalar::Int64(i as i64)).collect()).unwrap(),
    )
    .unwrap();
    // scattered positions
    let pos: Vec<i64> = (0..n).map(|i| (sm(i, 7) % n as u64) as i64).collect();
    timeit("datetime take 2M scattered", || {
        std::hint::black_box(dt.take(&pos).unwrap());
    });
    timeit("i64 take 2M scattered", || {
        std::hint::black_box(i64s.take(&pos).unwrap());
    });
}
