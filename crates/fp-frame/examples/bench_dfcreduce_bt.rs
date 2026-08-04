use std::collections::BTreeMap;

use fp_columnar::Column;
use fp_frame::{DataFrame, concat_dataframes_with_ignore_index};
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
fn mkcol(f: impl Fn(usize) -> Scalar, n: usize) -> Column {
    Column::from_values((0..n).map(f).collect()).unwrap()
}
fn main() {
    let n = 1_000_000usize;
    let dfs: Vec<DataFrame> = (0..5)
        .map(|k| {
            let idx = Index::from_range((k * n) as i64, ((k + 1) * n) as i64, 1);
            let mut m: BTreeMap<String, Column> = BTreeMap::new();
            m.insert(
                "a".into(),
                mkcol(|i| Scalar::Float64((i + k * n) as f64), n),
            );
            m.insert(
                "b".into(),
                mkcol(|i| Scalar::Float64(((i + k) as f64) * 0.5), n),
            );
            m.insert("c".into(), mkcol(|i| Scalar::Int64((i + k * n) as i64), n));
            DataFrame::new(idx, m).unwrap()
        })
        .collect();
    let refs: Vec<&DataFrame> = dfs.iter().collect();
    t("dfconcat+sum", || {
        let d = concat_dataframes_with_ignore_index(&refs, true).unwrap();
        std::hint::black_box(d.sum().unwrap());
    });
    t("dfconcat+mean", || {
        let d = concat_dataframes_with_ignore_index(&refs, true).unwrap();
        std::hint::black_box(d.mean().unwrap());
    });
    t("dfconcat+max", || {
        let d = concat_dataframes_with_ignore_index(&refs, true).unwrap();
        std::hint::black_box(d.max_agg().unwrap());
    });
}
