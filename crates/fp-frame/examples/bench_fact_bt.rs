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
    let fhc = Series::new(
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
    let iwide = Series::new(
        "v",
        idx.clone(),
        Column::from_values(
            (0..n)
                .map(|i| Scalar::Int64((sm(i, 7) % 2000000000) as i64))
                .collect(),
        )
        .unwrap(),
    )
    .unwrap();
    t("factorize_f64_hc", || {
        std::hint::black_box(fhc.factorize().unwrap());
    });
    t("factorize_i64_wide", || {
        std::hint::black_box(iwide.factorize().unwrap());
    });
    t("drop_dup_f64_hc", || {
        std::hint::black_box(fhc.drop_duplicates().unwrap());
    });
    t("unique_f64_hc", || {
        std::hint::black_box(fhc.unique());
    });
}
