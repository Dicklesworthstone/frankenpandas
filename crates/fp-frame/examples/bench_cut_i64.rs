use fp_columnar::Column;
use fp_frame::Series;
use fp_index::Index;
fn sm(i: usize, s: u64) -> u64 {
    let mut h = (i as u64).wrapping_add(s).wrapping_mul(0x9E3779B97F4A7C15);
    h = (h ^ (h >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    h ^ (h >> 31)
}
fn timeit(l: &str, mut f: impl FnMut()) {
    let mut b = u128::MAX;
    for _ in 0..6 {
        let t = std::time::Instant::now();
        f();
        b = b.min(t.elapsed().as_nanos());
    }
    println!("{l}: {:.2}ms", b as f64 / 1e6);
}
fn main() {
    let n = 3_000_000usize;
    let s = Series::new(
        "s",
        Index::from_range(0, n as i64, 1),
        Column::from_i64_values_owned((0..n).map(|i| (sm(i, 1) % 100000) as i64).collect()),
    )
    .unwrap();
    timeit("cut(i64,10) 3M", || {
        std::hint::black_box(fp_frame::cut(&s, 10).unwrap());
    });
}
