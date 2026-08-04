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
    for _ in 0..6 {
        let t = std::time::Instant::now();
        f();
        b = b.min(t.elapsed().as_nanos());
    }
    println!("{l}: {:.2}ms", b as f64 / 1e6);
}
fn mks(n: usize, off: i64) -> Series {
    let idx = Index::from_range(off, off + n as i64, 1);
    let v: Vec<Scalar> = (0..n)
        .map(|i| Scalar::Float64((sm(i, 1) % 1000) as f64))
        .collect();
    Series::new("s", idx, Column::from_values(v).unwrap()).unwrap()
}
fn main() {
    let n = 1_000_000usize;
    let a = mks(n, 0);
    let b_aligned = mks(n, 0);
    let b_shift = mks(n, 500_000);
    timeit("s.add aligned (1M)", || {
        std::hint::black_box(a.add(&b_aligned).unwrap());
    });
    timeit("s.add unaligned-shift (1M)", || {
        std::hint::black_box(a.add(&b_shift).unwrap());
    });
}
