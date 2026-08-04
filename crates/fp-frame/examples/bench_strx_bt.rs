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
    let n = 1_000_000usize;
    let idx = Index::from_range(0, n as i64, 1);
    let s_str = Series::new(
        "s",
        idx.clone(),
        Column::from_values(
            (0..n)
                .map(|i| Scalar::Utf8(format!("id{}x{}", sm(i, 7) % 100000, sm(i, 9) % 1000)))
                .collect(),
        )
        .unwrap(),
    )
    .unwrap();
    t("str_extract", || {
        std::hint::black_box(s_str.str().extract_df(r"id(\d+)x(\d+)").unwrap());
    });
    t("str_contains_re", || {
        std::hint::black_box(s_str.str().contains_regex(r"\d{3}x").unwrap());
    });
}
