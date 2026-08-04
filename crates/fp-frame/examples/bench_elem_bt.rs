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
    for _ in 0..8 {
        let x = std::time::Instant::now();
        f();
        b = b.min(x.elapsed().as_nanos());
    }
    println!("{l}: {:.2}ms", b as f64 / 1e6);
}
fn dig(s: &Series) -> u64 {
    let mut h = 1469598103934665603u64;
    for v in s.column().values().iter() {
        if let Scalar::Float64(x) = v {
            for by in x.to_bits().to_le_bytes() {
                h ^= by as u64;
                h = h.wrapping_mul(1099511628211);
            }
        }
    }
    h
}
fn main() {
    let n = 2_000_000usize;
    let idx = Index::from_range(0, n as i64, 1);
    let v = Series::new(
        "v",
        idx.clone(),
        Column::from_values(
            (0..n)
                .map(|i| Scalar::Float64(((sm(i, 7) % 100000) as f64 - 50000.0) * 0.5))
                .collect(),
        )
        .unwrap(),
    )
    .unwrap();
    println!("round_digest={}", dig(&v.round(2).unwrap()));
    t("round2", || {
        std::hint::black_box(v.round(2).unwrap());
    });
    t("round0", || {
        std::hint::black_box(v.round(0).unwrap());
    });
}
