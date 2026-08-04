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
fn digest(df: &fp_frame::DataFrame) -> u64 {
    let mut s = 0u64;
    for c in ["sum", "mean", "std", "var", "min", "max", "prod"] {
        if let Some(col) = df.column(c) {
            for v in col.values().iter() {
                if let Scalar::Float64(f) = v {
                    s = s.wrapping_mul(1099511628211).wrapping_add(f.to_bits());
                }
            }
        }
    }
    s
}
fn main() {
    let n = 1_000_000usize;
    let idx = Index::from_range(0, n as i64, 1);
    let k = Series::new(
        "k",
        idx.clone(),
        Column::from_values(
            (0..n)
                .map(|i| Scalar::Utf8(format!("k{}", sm(i, 1) % 500)))
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
    let ki = Series::new(
        "ki",
        idx.clone(),
        Column::from_values(
            (0..n)
                .map(|i| Scalar::Int64((sm(i, 1) % 500) as i64))
                .collect(),
        )
        .unwrap(),
    )
    .unwrap();
    t("agg_multi5_utf8", || {
        std::hint::black_box(
            v.groupby(&k)
                .unwrap()
                .agg(&["sum", "mean", "std", "min", "max"])
                .unwrap(),
        );
    });
    t("agg_multi5_i64", || {
        std::hint::black_box(
            v.groupby(&ki)
                .unwrap()
                .agg(&["sum", "mean", "std", "min", "max"])
                .unwrap(),
        );
    });
    let r = v
        .groupby(&k)
        .unwrap()
        .agg(&["sum", "mean", "std", "var", "min", "max", "prod"])
        .unwrap();
    println!("digest={}", digest(&r));
}
