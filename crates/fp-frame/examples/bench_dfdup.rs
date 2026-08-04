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
fn timeit(l: &str, mut f: impl FnMut()) {
    let mut b = u128::MAX;
    for _ in 0..5 {
        let t = std::time::Instant::now();
        f();
        b = b.min(t.elapsed().as_nanos());
    }
    println!("{l}: {:.2}ms", b as f64 / 1e6);
}
fn mkcol(n: usize, s: u64, card: u64) -> Column {
    Column::from_values(
        (0..n)
            .map(|i| Scalar::Int64(((sm(i, s) % card) as i64) * 2654435761))
            .collect(),
    )
    .unwrap()
}
fn main() {
    let n = 2_000_000usize;
    let idx = Index::from_range(0, n as i64, 1);
    // single wide-i64 col
    let mut m1 = BTreeMap::new();
    m1.insert("a".into(), mkcol(n, 1, n as u64));
    let df1 = DataFrame::new_with_column_order(idx.clone(), m1, vec!["a".into()]).unwrap();
    timeit("df.duplicated 1col wide-i64", || {
        std::hint::black_box(df1.duplicated(None, DuplicateKeep::First).unwrap());
    });
    timeit("df.drop_duplicates 1col wide-i64", || {
        std::hint::black_box(
            df1.drop_duplicates(None, DuplicateKeep::First, false)
                .unwrap(),
        );
    });
    // 2 wide-i64 cols
    let mut m2 = BTreeMap::new();
    m2.insert("a".into(), mkcol(n, 1, n as u64));
    m2.insert("b".into(), mkcol(n, 2, n as u64));
    let df2 =
        DataFrame::new_with_column_order(idx.clone(), m2, vec!["a".into(), "b".into()]).unwrap();
    timeit("df.duplicated 2col wide-i64", || {
        std::hint::black_box(df2.duplicated(None, DuplicateKeep::First).unwrap());
    });
    timeit("df.drop_duplicates 2col wide-i64", || {
        std::hint::black_box(
            df2.drop_duplicates(None, DuplicateKeep::First, false)
                .unwrap(),
        );
    });
}
