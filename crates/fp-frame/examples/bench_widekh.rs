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
    let idx = Index::from_range(0, n as i64, 1);
    // WIDE high-cardinality i64: ~half distinct, wide range (khash-floor case)
    let wv: Vec<Scalar> = (0..n)
        .map(|i| Scalar::Int64(((sm(i, 1) % (n as u64 / 2)) as i64) * 7919))
        .collect();
    let s = Series::new("s", idx.clone(), Column::from_values(wv).unwrap()).unwrap();
    timeit("value_counts WIDE-i64 high-card", || {
        std::hint::black_box(s.value_counts().unwrap());
    });
    timeit("factorize WIDE-i64 high-card", || {
        std::hint::black_box(s.factorize().unwrap());
    });
    timeit("nunique WIDE-i64 high-card", || {
        std::hint::black_box(s.nunique());
    });
    timeit("unique WIDE-i64 high-card", || {
        std::hint::black_box(s.unique());
    });
    timeit("duplicated WIDE-i64 high-card", || {
        std::hint::black_box(s.duplicated().unwrap());
    });
}
