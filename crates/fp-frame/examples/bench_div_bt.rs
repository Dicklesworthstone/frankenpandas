use std::collections::BTreeMap;

use fp_columnar::Column;
use fp_frame::{DataFrame, Series};
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
    let mut m = BTreeMap::new();
    let mut order = Vec::new();
    for c in 0..6 {
        let name = format!("c{c}");
        order.push(name.clone());
        m.insert(
            name,
            Column::from_values(
                (0..n)
                    .map(|i| Scalar::Float64((sm(i, (c as u64) * 13 + 7) % 100000) as f64))
                    .collect(),
            )
            .unwrap(),
        );
    }
    let df = DataFrame::new_with_column_order(idx.clone(), m, order).unwrap();
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
    t("corr_spearman_6col", || {
        std::hint::black_box(df.corr_method("spearman").unwrap());
    });
    t("rank_avg", || {
        std::hint::black_box(v.rank("average", true, "keep").unwrap());
    });
    t("rank_first", || {
        std::hint::black_box(v.rank("first", true, "keep").unwrap());
    });
}
