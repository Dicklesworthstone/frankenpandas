use fp_columnar::Column;
use fp_frame::{DataFrame, Series};
use fp_index::Index;
use fp_types::Scalar;
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
    let n = 500_000usize;
    let idx = Index::from_range(0, n as i64, 1);
    let c1 = Series::new(
        "c1",
        idx.clone(),
        Column::from_values(
            (0..n)
                .map(|i| Scalar::Utf8(format!("c{}", sm(i, 1) % 100)))
                .collect(),
        )
        .unwrap(),
    )
    .unwrap();
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
    t("gb_rank", || {
        std::hint::black_box(
            v.groupby(&c1)
                .unwrap()
                .rank("average", true, "keep")
                .unwrap(),
        );
    });
    t("gb_transform_mean", || {
        std::hint::black_box(v.groupby(&c1).unwrap().transform("mean").unwrap());
    });
    // crosstab + get_dummies
    let c2 = Series::new(
        "c2",
        idx.clone(),
        Column::from_values(
            (0..n)
                .map(|i| Scalar::Utf8(format!("d{}", sm(i, 2) % 80)))
                .collect(),
        )
        .unwrap(),
    )
    .unwrap();
    t("crosstab", || {
        std::hint::black_box(DataFrame::crosstab(&c1, &c2).unwrap());
    });
    t("factorize", || {
        std::hint::black_box(c1.factorize().unwrap());
    });
}
