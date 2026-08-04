use std::collections::BTreeMap;

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::Index;
use fp_types::{NullKind, Scalar};
fn sm(i: usize, s: u64) -> u64 {
    let mut h = (i as u64).wrapping_add(s).wrapping_mul(0x9E3779B97F4A7C15);
    h = (h ^ (h >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    h ^ (h >> 31)
}
fn mkf(off: i64, n: usize, ncol: usize) -> DataFrame {
    let idx = Index::from_range(off, off + n as i64, 1);
    let mut m = BTreeMap::new();
    let mut order = vec![];
    for c in 0..ncol {
        let nm = format!("c{c}");
        let v: Vec<Scalar> = (0..n)
            .map(|i| {
                if sm(i, c as u64 + 7).is_multiple_of(5) {
                    Scalar::Null(NullKind::Null)
                } else {
                    Scalar::Float64((sm(i, c as u64 + 1) % 100) as f64)
                }
            })
            .collect();
        m.insert(nm.clone(), Column::from_values(v).unwrap());
        order.push(nm);
    }
    DataFrame::new_with_column_order(idx, m, order).unwrap()
}
fn mkbool(off: i64, n: usize, ncol: usize) -> DataFrame {
    let idx = Index::from_range(off, off + n as i64, 1);
    let mut m = BTreeMap::new();
    let mut order = vec![];
    for c in 0..ncol {
        let nm = format!("c{c}");
        let v: Vec<Scalar> = (0..n)
            .map(|i| Scalar::Bool(sm(i, c as u64 + 3).is_multiple_of(2)))
            .collect();
        m.insert(nm.clone(), Column::from_values(v).unwrap());
        order.push(nm);
    }
    DataFrame::new_with_column_order(idx, m, order).unwrap()
}
fn dump(tag: &str, df: &DataFrame) {
    let idx = df.index().labels();
    print!("{tag} rows={} ", idx.len());
    for c in 0..3 {
        let col = df.column(&format!("c{c}")).unwrap().values();
        for i in 0..idx.len() {
            let key = match &idx[i] {
                fp_index::IndexLabel::Int64(x) => *x,
                _ => i64::MIN,
            };
            let val = match &col[i] {
                Scalar::Float64(f) => {
                    if f.is_nan() {
                        "NaN".into()
                    } else {
                        format!("{f:.4}")
                    }
                }
                Scalar::Null(_) => "NaN".into(),
                o => format!("{o:?}"),
            };
            print!("c{c}:{key}:{val} ");
        }
    }
    println!();
}
fn main() {
    // small unaligned: self [-5,55), cond/other [20,80) -> overlap [20,55)
    let n = 60usize;
    let ncol = 3;
    let s = mkf(-5, n, ncol);
    let cond = mkbool(20, n, ncol);
    let other = mkf(20, n, ncol);
    dump("where", &s.where_cond_df(&cond, &other).unwrap());
    dump("mask", &s.mask_df_other(&cond, &other).unwrap());
}
