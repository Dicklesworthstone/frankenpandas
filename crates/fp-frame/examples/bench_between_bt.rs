use fp_columnar::{Column, ValidityMask};
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
    for _ in 0..6 {
        let x = std::time::Instant::now();
        f();
        b = b.min(x.elapsed().as_nanos());
    }
    println!("{l}: {:.2}ms", b as f64 / 1e6);
}

fn between_nullable_i64_scalar(series: &Series, lo: f64, hi: f64) -> Series {
    let values: Vec<Scalar> = series
        .values()
        .iter()
        .map(|value| {
            Scalar::Bool(
                !value.is_missing() && value.to_f64().is_ok_and(|value| value >= lo && value <= hi),
            )
        })
        .collect();
    Series::new(
        "v",
        series.index().clone(),
        Column::from_values(values).unwrap(),
    )
    .unwrap()
}

fn main() {
    let n = 2_000_000usize;
    let idx = Index::from_range(0, n as i64, 1);
    let av: Vec<Scalar> = (0..n)
        .map(|i| Scalar::Float64((sm(i, 7) % 100000) as f64))
        .collect();
    let sa = Series::new("v", idx.clone(), Column::from_values(av).unwrap()).unwrap();
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
    let ni: Vec<i64> = (0..n).map(|i| (sm(i, 7) % 100000) as i64).collect();
    let mut ni_validity = ValidityMask::all_valid(n);
    for i in (0..n).step_by(5) {
        ni_validity.set(i, false);
    }
    let sni = Series::new(
        "v",
        idx.clone(),
        Column::from_i64_values_with_validity(ni, ni_validity),
    )
    .unwrap();
    let lo = Scalar::Float64(100.0);
    let hi = Scalar::Float64(90000.0);
    t("between_allvalid", || {
        std::hint::black_box(sa.between(&lo, &hi, "both").unwrap());
    });
    t("between_nullable", || {
        std::hint::black_box(sn.between(&lo, &hi, "both").unwrap());
    });
    t("between_nullable_i64", || {
        std::hint::black_box(sni.between(&lo, &hi, "both").unwrap());
    });
    t("between_nullable_i64_scalar", || {
        std::hint::black_box(between_nullable_i64_scalar(&sni, 100.0, 90000.0));
    });
    t("gt_allvalid", || {
        std::hint::black_box(sa.gt_scalar(&lo).unwrap());
    });
    t("gt_nullable", || {
        std::hint::black_box(sn.gt_scalar(&lo).unwrap());
    });
}
