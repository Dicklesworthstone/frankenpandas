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
    let v2 = Series::new(
        "v2",
        idx.clone(),
        Column::from_values(
            (0..n)
                .map(|i| {
                    if sm(i, 1).is_multiple_of(3) {
                        Scalar::Null(NullKind::NaN)
                    } else {
                        Scalar::Float64((sm(i, 9) % 100000) as f64)
                    }
                })
                .collect(),
        )
        .unwrap(),
    )
    .unwrap();
    let s_str = Series::new(
        "s",
        idx.clone(),
        Column::from_values(
            (0..n)
                .map(|i| Scalar::Utf8(format!("{}_{}", sm(i, 7) % 1000, sm(i, 9) % 1000)))
                .collect(),
        )
        .unwrap(),
    )
    .unwrap();
    t("str_split_expand", || {
        std::hint::black_box(s_str.str().split_df("_").unwrap());
    });
    t("duplicated", || {
        std::hint::black_box(v.duplicated().unwrap());
    });
    t("combine_first", || {
        std::hint::black_box(v2.combine_first(&v).unwrap());
    });
    t("fillna", || {
        std::hint::black_box(v2.fillna(&Scalar::Float64(0.0)).unwrap());
    });
    t("shift", || {
        std::hint::black_box(v.shift(1).unwrap());
    });
}
