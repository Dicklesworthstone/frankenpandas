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
fn mks(n: usize, off: i64, seed: u64, nullable: bool) -> Series {
    let idx = Index::from_range(off, off + n as i64, 1);
    let v: Vec<Scalar> = (0..n)
        .map(|i| {
            if nullable && sm(i, seed).is_multiple_of(4) {
                Scalar::Null(NullKind::Null)
            } else {
                Scalar::Float64((sm(i, seed) % 1000) as f64)
            }
        })
        .collect();
    Series::new("s", idx, Column::from_values(v).unwrap()).unwrap()
}
fn main() {
    let n = 1_000_000usize;
    let a = mks(n, 0, 1, true);
    let b_shift = mks(n, 500_000, 2, true);
    timeit("s.add_fill unaligned-shift nullable (1M)", || {
        std::hint::black_box(a.add_fill(&b_shift, 0.0).unwrap());
    });
    let a2 = mks(n, 0, 1, false);
    let b2 = mks(n, 500_000, 2, false);
    timeit("s.add_fill unaligned-shift all-valid (1M)", || {
        std::hint::black_box(a2.add_fill(&b2, 0.0).unwrap());
    });
    let b_al = mks(n, 0, 2, false);
    timeit("s.add_fill ALIGNED all-valid (1M) [isolate gather]", || {
        std::hint::black_box(a2.add_fill(&b_al, 0.0).unwrap());
    });
    timeit("s.add plain unaligned all-valid (1M) [ref]", || {
        std::hint::black_box(a2.add(&b2).unwrap());
    });
}
