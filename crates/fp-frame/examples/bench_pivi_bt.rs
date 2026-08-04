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
    let n = 500_000usize;
    let idx = Index::from_range(0, n as i64, 1);
    let mut m: BTreeMap<String, Column> = BTreeMap::new();
    m.insert("r".into(), col(|i| Scalar::Int64((i % 1000) as i64), n));
    m.insert(
        "c".into(),
        col(|i| Scalar::Int64((i / 1000 % 500) as i64), n),
    );
    m.insert("v".into(), col(|i| Scalar::Int64((i % 777) as i64), n)); // i64 values
    let df = DataFrame::new(idx, m).unwrap();
    t("pivot_i64vals", || {
        std::hint::black_box(df.pivot("r", "c", "v").unwrap());
    });
}
