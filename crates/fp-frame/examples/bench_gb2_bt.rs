use std::collections::BTreeMap;

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::Index;
use fp_types::Scalar;
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
fn kcol(n: usize, seed: u64, m: u64, pfx: &str) -> Column {
    Column::from_values(
        (0..n)
            .map(|i| Scalar::Utf8(format!("{pfx}{}", sm(i, seed) % m)))
            .collect(),
    )
    .unwrap()
}
fn vcol(n: usize) -> Column {
    Column::from_values(
        (0..n)
            .map(|i| Scalar::Float64((sm(i, 7) % 100000) as f64))
            .collect(),
    )
    .unwrap()
}
fn main() {
    let n = 1_000_000usize;
    // Frame A: single Utf8 key + single f64 value
    let mut a = BTreeMap::new();
    a.insert("k1".into(), kcol(n, 1, 50, "k"));
    a.insert("v".into(), vcol(n));
    let dfa = DataFrame::new_with_column_order(
        Index::from_range(0, n as i64, 1),
        a,
        vec!["k1".into(), "v".into()],
    )
    .unwrap();
    // Frame B: two Utf8 keys + single f64 value
    let mut b = BTreeMap::new();
    b.insert("k1".into(), kcol(n, 1, 50, "k"));
    b.insert("k2".into(), kcol(n, 2, 40, "g"));
    b.insert("v".into(), vcol(n));
    let dfb = DataFrame::new_with_column_order(
        Index::from_range(0, n as i64, 1),
        b,
        vec!["k1".into(), "k2".into(), "v".into()],
    )
    .unwrap();
    t("gb1u_sum", || {
        std::hint::black_box(dfa.groupby(&["k1"]).unwrap().sum().unwrap());
    });
    t("gb1u_mean", || {
        std::hint::black_box(dfa.groupby(&["k1"]).unwrap().mean().unwrap());
    });
    t("gb1u_std", || {
        std::hint::black_box(dfa.groupby(&["k1"]).unwrap().std().unwrap());
    });
    t("gb2_sum", || {
        std::hint::black_box(dfb.groupby(&["k1", "k2"]).unwrap().sum().unwrap());
    });
}
