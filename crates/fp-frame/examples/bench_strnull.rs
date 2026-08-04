//! str ops on a NULLABLE Utf8 series (~10% missing). bench_strnull <n>
use fp_frame::Series;
use fp_index::IndexLabel;
use fp_types::Scalar;
fn sm(i: usize, s: u64) -> u64 {
    let mut h = (i as u64).wrapping_add(s).wrapping_mul(0x9E3779B97F4A7C15);
    h = (h ^ (h >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    h ^ (h >> 31)
}
fn timeit<F: FnMut()>(l: &str, mut f: F) {
    let mut b = u128::MAX;
    for _ in 0..6 {
        let t = std::time::Instant::now();
        f();
        b = b.min(t.elapsed().as_nanos());
    }
    println!("{l}: {:.2}ms", b as f64 / 1e6);
}
fn main() {
    let n: usize = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(2_000_000);
    let words = [
        "apple",
        "Banana",
        "cherry",
        "DATE",
        "elderberry",
        "Fig",
        "grape",
    ];
    let vals: Vec<Scalar> = (0..n)
        .map(|i| {
            if sm(i, 3).is_multiple_of(10) {
                Scalar::Null(fp_types::NullKind::Null)
            } else {
                Scalar::Utf8(format!(
                    "{}_{}",
                    words[(sm(i, 0) % 7) as usize],
                    sm(i, 1) % 1000
                ))
            }
        })
        .collect();
    let labels: Vec<IndexLabel> = (0..n as i64).map(IndexLabel::Int64).collect();
    let s = Series::from_values("s", labels, vals).unwrap();
    timeit("upper", || {
        std::hint::black_box(s.str().upper().unwrap().len());
    });
    timeit("lower", || {
        std::hint::black_box(s.str().lower().unwrap().len());
    });
    timeit("len", || {
        std::hint::black_box(s.str().len().unwrap().len());
    });
    timeit("contains", || {
        std::hint::black_box(s.str().contains("an").unwrap().len());
    });
    timeit("startswith", || {
        std::hint::black_box(s.str().startswith("app").unwrap().len());
    });
}
