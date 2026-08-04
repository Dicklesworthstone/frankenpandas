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
    let n = 2_000_000usize;
    let idx = Index::from_range(0, n as i64, 1);
    let vn = Series::new(
        "v",
        idx.clone(),
        Column::from_values(
            (0..n)
                .map(|i| {
                    if sm(i, 1).is_multiple_of(5) {
                        Scalar::Null(NullKind::NaN)
                    } else {
                        Scalar::Float64((sm(i, 9) % 1000000) as f64)
                    }
                })
                .collect(),
        )
        .unwrap(),
    )
    .unwrap();
    let positions: Vec<i64> = (0..n).map(|i| (sm(i, 3) % n as u64) as i64).collect();
    t("take_nan", || {
        std::hint::black_box(vn.take(&positions).unwrap());
    });
    t("sort_values_nan", || {
        std::hint::black_box(vn.sort_values_na(true, "last").unwrap());
    });
    t("drop_duplicates_nan", || {
        std::hint::black_box(vn.drop_duplicates().unwrap());
    });
}
