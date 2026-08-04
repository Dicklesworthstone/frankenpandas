use std::collections::BTreeMap;

use fp_columnar::Column;
use fp_frame::{DataFrame, concat_dataframes_with_ignore_index};
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
fn dig(df: &DataFrame) -> u64 {
    let mut h = 1469598103934665603u64;
    for v in df.column("v").unwrap().values().iter() {
        let b = format!("{:?}", v);
        for by in b.bytes() {
            h ^= by as u64;
            h = h.wrapping_mul(1099511628211);
        }
    }
    h
}
fn main() {
    let n = 1_000_000usize;
    let idx = Index::from_range(0, n as i64, 1);
    let mk = |vf: bool| {
        let mut m: BTreeMap<String, Column> = BTreeMap::new();
        m.insert(
            "v".into(),
            if vf {
                col(|i| Scalar::Float64(i as f64), n)
            } else {
                col(|i| Scalar::Int64(i as i64), n)
            },
        );
        m.insert("w".into(), col(|i| Scalar::Int64(i as i64), n));
        DataFrame::new(idx.clone(), m).unwrap()
    };
    let df1 = mk(true);
    let df2 = mk(false);
    let df3 = mk(false);
    let refs = vec![&df1, &df2, &df3];
    let r = concat_dataframes_with_ignore_index(&refs, true).unwrap();
    println!(
        "dtype={:?} digest={}",
        r.column("v").unwrap().dtype(),
        dig(&r)
    );
    t("dfconcat_mixedcol", || {
        std::hint::black_box(concat_dataframes_with_ignore_index(&refs, true).unwrap());
    });
}
