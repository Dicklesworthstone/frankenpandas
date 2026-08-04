use std::collections::BTreeMap;

use fp_columnar::Column;
use fp_frame::DataFrame;
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
fn dig(df: &DataFrame) -> u64 {
    let mut h = 1469598103934665603u64;
    for c in df.column_names().into_iter() {
        for v in df.column(c.as_str()).unwrap().values().iter() {
            let b = format!("{:?}", v);
            for by in b.bytes() {
                h ^= by as u64;
                h = h.wrapping_mul(1099511628211);
            }
        }
    }
    h
}
fn main() {
    let n = 500_000usize;
    let idx = Index::from_range(0, n as i64, 1);
    let mk = || {
        let mut m: BTreeMap<String, Column> = BTreeMap::new();
        m.insert("a".into(), col(|i| Scalar::Int64(i as i64), n));
        m.insert("b".into(), col(|i| Scalar::Float64(i as f64 * 0.5), n));
        m.insert("c".into(), col(|i| Scalar::Int64((i % 100) as i64), n));
        m.insert("d".into(), col(|i| Scalar::Int64((i % 7) as i64), n));
        DataFrame::new(idx.clone(), m).unwrap()
    };
    let df = mk();
    println!(
        "mixed_digest={}",
        dig(&df.melt(&["a"], &["b", "c", "d"], None, None).unwrap())
    );
    println!(
        "alli64_digest={}",
        dig(&df.melt(&["b"], &["a", "c", "d"], None, None).unwrap())
    );
    t("melt_mixed", || {
        std::hint::black_box(df.melt(&["a"], &["b", "c", "d"], None, None).unwrap());
    });
    t("melt_alli64", || {
        std::hint::black_box(df.melt(&["b"], &["a", "c", "d"], None, None).unwrap());
    });
}
