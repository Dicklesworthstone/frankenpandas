use fp_columnar::Column;
use fp_frame::Series;
use fp_index::Index;
use fp_types::{NullKind, Scalar};
fn sm(i: usize, s: u64) -> u64 {
    let mut h = (i as u64).wrapping_add(s).wrapping_mul(0x9E3779B97F4A7C15);
    h = (h ^ (h >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    h ^ (h >> 31)
}
fn t(l: &str, mut f: impl FnMut()) {
    let mut b = u128::MAX;
    for _ in 0..4 {
        let x = std::time::Instant::now();
        f();
        b = b.min(x.elapsed().as_nanos());
    }
    println!("{l}: {:.2}ms", b as f64 / 1e6);
}
fn main() {
    let n = 5_000_000usize;
    let idx = Index::from_range(0, n as i64, 1);
    let v = Series::new(
        "v",
        idx.clone(),
        Column::from_values(
            (0..n)
                .map(|i| Scalar::Float64((sm(i, 7) % 100000) as f64))
                .collect(),
        )
        .unwrap(),
    )
    .unwrap();
    let vn = Series::new(
        "vn",
        idx.clone(),
        Column::from_values(
            (0..n)
                .map(|i| {
                    if sm(i, 1).is_multiple_of(10) {
                        Scalar::Null(NullKind::NaN)
                    } else {
                        Scalar::Float64((sm(i, 7) % 100000) as f64)
                    }
                })
                .collect(),
        )
        .unwrap(),
    )
    .unwrap();
    t("skew", || {
        std::hint::black_box(v.skew().unwrap());
    });
    t("kurt", || {
        std::hint::black_box(v.kurtosis().unwrap());
    });
    t("sem", || {
        std::hint::black_box(v.sem().unwrap());
    });
    t("std", || {
        std::hint::black_box(v.std().unwrap());
    });
    t("var", || {
        std::hint::black_box(v.var().unwrap());
    });
    t("nunique", || {
        std::hint::black_box(v.nunique());
    });
    t("skew_nan", || {
        std::hint::black_box(vn.skew().unwrap());
    });
    t("std_nan", || {
        std::hint::black_box(vn.std().unwrap());
    });
}
