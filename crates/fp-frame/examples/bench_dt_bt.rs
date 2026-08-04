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
    for _ in 0..3 {
        let x = std::time::Instant::now();
        f();
        b = b.min(x.elapsed().as_nanos());
    }
    println!("{l}: {:.2}ms", b as f64 / 1e6);
}
fn main() {
    let n = 2_000_000usize;
    let idx = Index::from_range(0, n as i64, 1);
    let base: i64 = 1_420_070_400_000_000_000; // 2015-01-01 ns
    let s = Series::new(
        "t",
        idx.clone(),
        Column::from_values(
            (0..n)
                .map(|i| {
                    Scalar::Datetime64(
                        base + (sm(i, 3) % (6 * 365 * 24 * 3600)) as i64 * 1_000_000_000,
                    )
                })
                .collect(),
        )
        .unwrap(),
    )
    .unwrap();
    t("dt_year", || {
        std::hint::black_box(s.dt().year().unwrap());
    });
    t("dt_dayofweek", || {
        std::hint::black_box(s.dt().dayofweek().unwrap());
    });
    t("dt_floor_D", || {
        std::hint::black_box(s.dt().floor("D").unwrap());
    });
    t("dt_normalize", || {
        std::hint::black_box(s.dt().normalize().unwrap());
    });
}
