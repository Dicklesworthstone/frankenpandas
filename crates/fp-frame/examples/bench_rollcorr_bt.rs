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
    let n = 1_000_000usize;
    let a: Vec<Scalar> = (0..n)
        .map(|i| Scalar::Float64((sm(i, 7) % 100000) as f64))
        .collect();
    let b: Vec<Scalar> = (0..n)
        .map(|i| Scalar::Float64((sm(i, 9) % 100000) as f64))
        .collect();
    let sa = Series::new(
        "a",
        Index::from_range(0, n as i64, 1),
        Column::from_values(a).unwrap(),
    )
    .unwrap();
    let sb = Series::new(
        "b",
        Index::from_range(0, n as i64, 1),
        Column::from_values(b).unwrap(),
    )
    .unwrap();
    for w in [100usize, 1000] {
        t(&format!("roll_corr_w{w}"), || {
            std::hint::black_box(sa.rolling(w, None).corr(&sb).unwrap());
        });
        t(&format!("roll_cov_w{w}"), || {
            std::hint::black_box(sa.rolling(w, None).cov(&sb).unwrap());
        });
    }
}
