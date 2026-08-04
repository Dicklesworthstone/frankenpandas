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
    for _ in 0..6 {
        let x = std::time::Instant::now();
        f();
        b = b.min(x.elapsed().as_nanos());
    }
    println!("{l}: {:.2}ms", b as f64 / 1e6);
}
fn main() {
    let n = 2_000_000usize;
    let vv: Vec<Scalar> = (0..n)
        .map(|i| {
            if sm(i, 1).is_multiple_of(5) {
                Scalar::Null(NullKind::NaN)
            } else {
                Scalar::Float64((sm(i, 7) % 100000) as f64)
            }
        })
        .collect();
    let s = Series::new(
        "v",
        Index::from_range(0, n as i64, 1),
        Column::from_values(vv).unwrap(),
    )
    .unwrap();
    t("nlargest20", || {
        std::hint::black_box(s.nlargest(20).unwrap());
    });
    t("nsmallest20", || {
        std::hint::black_box(s.nsmallest(20).unwrap());
    });
    t("rank", || {
        std::hint::black_box(s.rank("average", true, "keep").unwrap());
    });
    t("clip", || {
        std::hint::black_box(s.clip(Some(100.0), Some(90000.0)).unwrap());
    });
    t("round", || {
        std::hint::black_box(s.round(1).unwrap());
    });
    t("value_counts", || {
        std::hint::black_box(s.value_counts().unwrap());
    });
    t("diff", || {
        std::hint::black_box(s.diff(1).unwrap());
    });
    t("abs", || {
        std::hint::black_box(s.abs().unwrap());
    });
    t("cumsum", || {
        std::hint::black_box(s.cumsum().unwrap());
    });
    t("between", || {
        std::hint::black_box(
            s.between(&Scalar::Float64(100.0), &Scalar::Float64(90000.0), "both")
                .unwrap(),
        );
    });
}
