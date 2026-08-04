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
fn dig(s: &Series) -> u64 {
    let r = s.value_counts().unwrap();
    let mut d = 0u64;
    for v in r.column().values().iter() {
        if let Scalar::Int64(c) = v {
            d = d.wrapping_mul(1099511628211).wrapping_add(*c as u64);
        }
    }
    for l in r.index().labels().iter() {
        if let fp_index::IndexLabel::Float64(f) = l {
            d = d.wrapping_mul(1099511628211).wrapping_add(f.0.to_bits());
        }
    }
    d
}
fn main() {
    let n = 2_000_000usize;
    let idx = Index::from_range(0, n as i64, 1);
    let hc = Series::new(
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
    let lc = Series::new(
        "v",
        idx.clone(),
        Column::from_values(
            (0..n)
                .map(|i| Scalar::Float64((sm(i, 7) % 500) as f64))
                .collect(),
        )
        .unwrap(),
    )
    .unwrap();
    println!("vc_digest_hc={} vc_digest_lc={}", dig(&hc), dig(&lc));
    t("vc_highcard", || {
        std::hint::black_box(hc.value_counts().unwrap());
    });
    t("vc_lowcard", || {
        std::hint::black_box(lc.value_counts().unwrap());
    });
}
