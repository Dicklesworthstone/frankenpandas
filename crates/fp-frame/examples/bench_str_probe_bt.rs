use fp_columnar::Column;
use fp_frame::Series;
use fp_index::Index;
use fp_types::Scalar;
fn sm(i: usize, s: u64) -> u64 {
    let mut h = (i as u64).wrapping_add(s).wrapping_mul(0x9E3779B97F4A7C15);
    h = (h ^ (h >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    h ^ (h >> 31)
}
fn t(l: &str, mut f: impl FnMut()) {
    let mut b = u128::MAX;
    for _ in 0..5 {
        let x = std::time::Instant::now();
        f();
        b = b.min(x.elapsed().as_nanos());
    }
    println!("{l}: {:.2}ms", b as f64 / 1e6);
}
fn main() {
    let n = 1_000_000usize;
    let words = [
        "alpha",
        "beta",
        "gamma",
        "delta",
        "Epsilon",
        "ZETA",
        "eta_theta",
        "iota123",
        "KappaLambda",
        "mu",
    ];
    let vv: Vec<Scalar> = (0..n)
        .map(|i| {
            Scalar::Utf8(format!(
                "{}{}",
                words[(sm(i, 7) % words.len() as u64) as usize],
                sm(i, 3) % 1000
            ))
        })
        .collect();
    let s = Series::new(
        "v",
        Index::from_range(0, n as i64, 1),
        Column::from_values(vv).unwrap(),
    )
    .unwrap();
    t("len", || {
        std::hint::black_box(s.str().len().unwrap());
    });
    t("lower", || {
        std::hint::black_box(s.str().lower().unwrap());
    });
    t("upper", || {
        std::hint::black_box(s.str().upper().unwrap());
    });
    t("contains", || {
        std::hint::black_box(s.str().contains("eta").unwrap());
    });
    t("startswith", || {
        std::hint::black_box(s.str().startswith("al").unwrap());
    });
    t("endswith", || {
        std::hint::black_box(s.str().endswith("5").unwrap());
    });
    t("replace", || {
        std::hint::black_box(s.str().replace("a", "X").unwrap());
    });
    t("strip", || {
        std::hint::black_box(s.str().strip().unwrap());
    });
    t("capitalize", || {
        std::hint::black_box(s.str().capitalize().unwrap());
    });
    t("title", || {
        std::hint::black_box(s.str().title().unwrap());
    });
}
