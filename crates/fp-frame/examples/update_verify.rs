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
                    if sm(i, nm).is_multiple_of(4) {
                        Scalar::Null(NullKind::Null)
                    } else {
                        Scalar::Float64((sm(i, seed) % 100) as f64)
                    }
                })
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
    let a = mkdf(vec![("x".into(), dcol(1, 7)), ("y".into(), dcol(2, 8))]);
    let b = mkdf(vec![("x".into(), dcol(5, 9)), ("y".into(), dcol(6, 10))]);
    let r = a.update(&b).unwrap();
    let mut f = std::fs::File::create("/tmp/fp_update.txt").unwrap();
    let g = |df: &DataFrame, nm: &str| {
        df.column(nm)
            .unwrap()
            .values()
            .iter()
            .map(|v| match v {
                Scalar::Float64(x) => {
                    if x.is_nan() {
                        "NA".into()
                    } else {
                        format!("{}", *x as i64)
                    }
                }
                Scalar::Null(_) => "NA".into(),
                o => format!("{o:?}"),
            })
            .collect::<Vec<_>>()
            .join(",")
    };
    for nm in ["x", "y"] {
        writeln!(f, "A_{nm}\t{}", g(&a, nm)).unwrap();
        writeln!(f, "B_{nm}\t{}", g(&b, nm)).unwrap();
        writeln!(f, "R_{nm}\t{}", g(&r, nm)).unwrap();
    }
    println!("wrote");
}
