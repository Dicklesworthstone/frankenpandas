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
    for _ in 0..5 {
        let x = std::time::Instant::now();
        f();
        b = b.min(x.elapsed().as_nanos());
    }
    println!("{l}: {:.2}ms", b as f64 / 1e6);
}
fn main() {
    let n = 2_000_000usize;
    let idx = Index::from_range(0, n as i64, 1);
    // all-valid, high-card (~100k distinct)
    let av: Vec<Scalar> = (0..n)
        .map(|i| Scalar::Float64((sm(i, 7) % 100000) as f64))
        .collect();
    let sa = Series::new("v", idx.clone(), Column::from_values(av).unwrap()).unwrap();
    // nullable 20%, same distinct
    let nv: Vec<Scalar> = (0..n)
        .map(|i| {
            if sm(i, 1).is_multiple_of(5) {
                Scalar::Null(NullKind::NaN)
            } else {
                Scalar::Float64((sm(i, 7) % 100000) as f64)
            }
        })
        .collect();
    let sn = Series::new("v", idx.clone(), Column::from_values(nv).unwrap()).unwrap();
    // low-card variants (~500 distinct)
    let al: Vec<Scalar> = (0..n)
        .map(|i| Scalar::Float64((sm(i, 7) % 500) as f64))
        .collect();
    let sal = Series::new("v", idx.clone(), Column::from_values(al).unwrap()).unwrap();
    let nl: Vec<Scalar> = (0..n)
        .map(|i| {
            if sm(i, 1).is_multiple_of(5) {
                Scalar::Null(NullKind::NaN)
            } else {
                Scalar::Float64((sm(i, 7) % 500) as f64)
            }
        })
        .collect();
    let snl = Series::new("v", idx.clone(), Column::from_values(nl).unwrap()).unwrap();
    t("vc_allvalid_highcard", || {
        std::hint::black_box(sa.value_counts().unwrap());
    });
    t("vc_nullable_highcard", || {
        std::hint::black_box(sn.value_counts().unwrap());
    });
    t("vc_allvalid_lowcard", || {
        std::hint::black_box(sal.value_counts().unwrap());
    });
    t("vc_nullable_lowcard", || {
        std::hint::black_box(snl.value_counts().unwrap());
    });
}
