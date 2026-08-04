use std::collections::BTreeMap;

use fp_columnar::Column;
use fp_frame::{DataFrame, Series};
use fp_index::Index;
use fp_types::Scalar;
fn t(l: &str, mut f: impl FnMut()) {
    let mut b = u128::MAX;
    for _ in 0..5 {
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
    m.insert("a".into(), col(|i| Scalar::Int64(i as i64), n));
    m.insert("b".into(), col(|i| Scalar::Float64(i as f64 * 0.5), n));
    let df = DataFrame::new(idx.clone(), m).unwrap();
    let sc = Series::new(
        "c",
        idx.clone(),
        col(|i| Scalar::Int64((i % 100) as i64), n),
    )
    .unwrap();
    t("cumsum_df", || {
        std::hint::black_box(df.cumsum().unwrap());
    });
    t("pct_change_df", || {
        std::hint::black_box(df.pct_change(1).unwrap());
    });
    t("rank_int64", || {
        std::hint::black_box(sc.rank("average", true, "keep").unwrap());
    });
}
