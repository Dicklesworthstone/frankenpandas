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
    let words = [
        "apple",
        "Banana",
        "cherry_PIE",
        "date fruit",
        "Elderberry",
        "fig",
        "grape123",
    ];
    let sv: Vec<Scalar> = (0..n)
        .map(|i| {
            Scalar::Utf8(format!(
                "{}_{}",
                words[(sm(i, 1) % 7) as usize],
                sm(i, 2) % 1000
            ))
        })
        .collect();
    let idx = Index::from_range(0, n as i64, 1);
    let s = Series::new("s", idx, Column::from_values(sv).unwrap()).unwrap();
    timeit("str.lower", || {
        std::hint::black_box(s.str().lower().unwrap());
    });
    timeit("str.upper", || {
        std::hint::black_box(s.str().upper().unwrap());
    });
    timeit("str.len", || {
        std::hint::black_box(s.str().len().unwrap());
    });
    timeit("str.contains(lit)", || {
        std::hint::black_box(s.str().contains("fruit").unwrap());
    });
    timeit("str.startswith", || {
        std::hint::black_box(s.str().startswith("Ban").unwrap());
    });
    timeit("str.endswith", || {
        std::hint::black_box(s.str().endswith("9").unwrap());
    });
    timeit("str.replace", || {
        std::hint::black_box(s.str().replace("_", "-").unwrap());
    });
    timeit("str.strip", || {
        std::hint::black_box(s.str().strip().unwrap());
    });
}
