use fp_columnar::Column;
use fp_frame::Series;
use fp_index::Index;
use fp_types::{NullKind, Scalar};
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
fn mkf(off: i64, n: usize, seed: u64, nullable: bool) -> Series {
    let idx = Index::from_range(off, off + n as i64, 1);
    let v: Vec<Scalar> = (0..n)
        .map(|i| {
            if nullable && sm(i, seed).is_multiple_of(5) {
                Scalar::Null(NullKind::Null)
            } else {
                Scalar::Float64((sm(i, seed) % 100) as f64)
            }
        })
        .collect();
    Series::new("s", idx, Column::from_values(v).unwrap()).unwrap()
}
fn mkbool(off: i64, n: usize) -> Series {
    let idx = Index::from_range(off, off + n as i64, 1);
    let v: Vec<Scalar> = (0..n)
        .map(|i| Scalar::Bool(sm(i, 3).is_multiple_of(2)))
        .collect();
    Series::new("c", idx, Column::from_values(v).unwrap()).unwrap()
}
fn main() {
    let n = 1_000_000usize;
    let cond = mkbool(200_000, n);
    let s = mkf(0, n, 1, true);
    let other = mkf(200_000, n, 2, true);
    timeit("s.where UNALIGNED nullable (1M)", || {
        std::hint::black_box(s.where_cond_series(&cond, &other).unwrap());
    });
    let s2 = mkf(0, n, 1, false);
    let other2 = mkf(200_000, n, 2, false);
    timeit("s.where UNALIGNED all-valid (1M)", || {
        std::hint::black_box(s2.where_cond_series(&cond, &other2).unwrap());
    });
}
