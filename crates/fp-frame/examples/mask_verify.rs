use std::{collections::BTreeMap, io::Write};

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::Index;
use fp_types::{NullKind, Scalar};
fn sm(i: usize, s: u64) -> u64 {
    let mut h = (i as u64).wrapping_add(s).wrapping_mul(0x9E3779B97F4A7C15);
    h = (h ^ (h >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    h ^ (h >> 31)
}
fn main() {
    let n = 3000usize;
    let idx = Index::from_range(0, n as i64, 1);
    let dcol = |seed: u64, nm: u64| -> Column {
        Column::from_values(
            (0..n)
                .map(|i| {
                    if sm(i, nm).is_multiple_of(5) {
                        Scalar::Null(NullKind::Null)
                    } else {
                        Scalar::Float64((sm(i, seed) % 100) as f64)
                    }
                })
                .collect(),
        )
        .unwrap()
    };
    let ccol = |seed: u64| -> Column {
        Column::from_values(
            (0..n)
                .map(|i| Scalar::Bool(sm(i, seed).is_multiple_of(2)))
                .collect(),
        )
        .unwrap()
    };
    let mkdf = |cols: Vec<(String, Column)>| {
        let mut m = BTreeMap::new();
        let mut o = vec![];
        for (nm, c) in cols {
            m.insert(nm.clone(), c);
            o.push(nm);
        }
        DataFrame::new_with_column_order(idx.clone(), m, o).unwrap()
    };
    let df = mkdf(vec![("a".into(), dcol(1, 7)), ("b".into(), dcol(2, 8))]);
    let cond = mkdf(vec![("a".into(), ccol(3)), ("b".into(), ccol(4))]);
    let other = mkdf(vec![("a".into(), dcol(5, 9)), ("b".into(), dcol(6, 10))]);
    let r = df.r#where(&cond, None).ok();
    let _ = r; // ensure method exists
    let res = df.mask_df_other(&cond, &other).unwrap();
    let mut f = std::fs::File::create("/tmp/fp_mask.txt").unwrap();
    let g = |df: &DataFrame, nm: &str| {
        let c = df.column(nm).unwrap().values();
        c.iter()
            .map(|v| match v {
                Scalar::Float64(x) => {
                    if x.is_nan() {
                        "NA".into()
                    } else {
                        format!("{}", *x as i64)
                    }
                }
                Scalar::Null(_) => "NA".into(),
                Scalar::Bool(b) => {
                    if *b {
                        "1".into()
                    } else {
                        "0".into()
                    }
                }
                o => format!("{o:?}"),
            })
            .collect::<Vec<_>>()
            .join(",")
    };
    for nm in ["a", "b"] {
        writeln!(f, "D_{nm}\t{}", g(&df, nm)).unwrap();
        writeln!(f, "C_{nm}\t{}", g(&cond, nm)).unwrap();
        writeln!(f, "O_{nm}\t{}", g(&other, nm)).unwrap();
        writeln!(f, "R_{nm}\t{}", g(&res, nm)).unwrap();
    }
    println!("wrote");
}
