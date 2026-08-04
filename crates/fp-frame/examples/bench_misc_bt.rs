use std::collections::BTreeMap;

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::{DuplicateKeep, Index};
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
    let mut m = BTreeMap::new();
    m.insert(
        "k1".to_string(),
        Column::from_values(
            (0..n)
                .map(|i| Scalar::Utf8(format!("k{}", sm(i, 1) % 50000)))
                .collect(),
        )
        .unwrap(),
    );
    m.insert(
        "i1".to_string(),
        Column::from_values(
            (0..n)
                .map(|i| Scalar::Int64((sm(i, 1) % 50000) as i64))
                .collect(),
        )
        .unwrap(),
    );
    m.insert(
        "v".to_string(),
        Column::from_values(
            (0..n)
                .map(|i| Scalar::Float64((sm(i, 7) % 100000) as f64))
                .collect(),
        )
        .unwrap(),
    );
    let df = DataFrame::new_with_column_order(
        Index::from_range(0, n as i64, 1),
        m,
        vec!["k1".into(), "i1".into(), "v".into()],
    )
    .unwrap();
    t("sort_str", || {
        std::hint::black_box(df.sort_values("k1", true).unwrap());
    });
    t("sort_2col", || {
        std::hint::black_box(
            df.sort_values_multi(&["i1", "v"], &[true, true], "last")
                .unwrap(),
        );
    });
    t("dup_str", || {
        std::hint::black_box(
            df.duplicated(Some(&["k1".to_string()]), DuplicateKeep::First)
                .unwrap(),
        );
    });
    t("dropdup_2col", || {
        std::hint::black_box(
            df.drop_duplicates(
                Some(&["k1".to_string(), "i1".to_string()]),
                DuplicateKeep::First,
                false,
            )
            .unwrap(),
        );
    });
    t("nunique", || {
        std::hint::black_box(df.nunique().unwrap());
    });
    let needles: Vec<Scalar> = (0..1000).map(|j| Scalar::Utf8(format!("k{j}"))).collect();
    let k1 = df.column("k1").unwrap().clone();
    let ks = fp_frame::Series::new("k1", df.index().clone(), k1).unwrap();
    t("isin_str", || {
        std::hint::black_box(ks.isin(&needles).unwrap());
    });
}
