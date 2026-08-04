use std::collections::BTreeMap;

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::Index;
use fp_types::Scalar;
fn t(l: &str, mut f: impl FnMut()) {
    let mut b = u128::MAX;
    for _ in 0..4 {
        let x = std::time::Instant::now();
        f();
        b = b.min(x.elapsed().as_nanos());
    }
    println!("{l}: {:.2}ms", b as f64 / 1e6);
}
fn col(f: impl Fn(usize) -> Scalar, n: usize) -> Column {
    Column::from_values((0..n).map(f).collect()).unwrap()
}
fn main() {
    let n = 1_000_000usize;
    let idx = Index::from_range(0, n as i64, 1);
    let mut m: BTreeMap<String, Column> = BTreeMap::new();
    m.insert("k1".into(), col(|i| Scalar::Int64((i % 100) as i64), n));
    m.insert("k2".into(), col(|i| Scalar::Int64((i % 50) as i64), n));
    m.insert("v1".into(), col(|i| Scalar::Float64(i as f64 * 0.5), n)); // f64
    m.insert("v2".into(), col(|i| Scalar::Int64((i % 1000) as i64), n)); // i64  -> MIXED
    let df = DataFrame::new(idx, m).unwrap();
    t("gb2_sum_mixed", || {
        std::hint::black_box(df.groupby(&["k1", "k2"]).unwrap().sum().unwrap());
    });
    t("gb2_mean_mixed", || {
        std::hint::black_box(df.groupby(&["k1", "k2"]).unwrap().mean().unwrap());
    });
}
