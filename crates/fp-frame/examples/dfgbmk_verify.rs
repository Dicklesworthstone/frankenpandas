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
fn dumpdf(f: &mut std::fs::File, tag: &str, df: &DataFrame) {
    let idx = df.index();
    let lbls = idx.labels();
    for cn in ["a", "b"] {
        let col = df.column(cn).unwrap().values();
        let mut rows: Vec<(String, String)> = Vec::new();
        for i in 0..idx.len() {
            let lbl = match &lbls[i] {
                fp_index::IndexLabel::Utf8(v) => v.clone(),
                o => format!("{o:?}"),
            };
            let v = match &col[i] {
                Scalar::Int64(x) => format!("i{x}"),
                Scalar::Float64(x) => format!("f{x:.4}"),
                Scalar::Null(_) => "NA".into(),
                o => format!("{o:?}"),
            };
            rows.push((lbl, v));
        }
        rows.sort();
        writeln!(
            f,
            "{tag}::{cn}\t{}",
            rows.iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join("|")
        )
        .unwrap();
    }
}
fn main() {
    let n = 5000usize;
    let k1: Vec<Scalar> = (0..n)
        .map(|i| Scalar::Int64((sm(i, 1) % 6) as i64))
        .collect();
    let k2: Vec<Scalar> = (0..n)
        .map(|i| Scalar::Int64((sm(i, 2) % 5) as i64))
        .collect();
    let a: Vec<Scalar> = (0..n)
        .map(|i| {
            if sm(i, 7).is_multiple_of(4) {
                Scalar::Null(NullKind::Null)
            } else {
                Scalar::Float64((sm(i, 9) % 100) as f64 - 50.0)
            }
        })
        .collect();
    let b: Vec<Scalar> = (0..n)
        .map(|i| {
            if sm(i, 3).is_multiple_of(3) {
                Scalar::Null(NullKind::Null)
            } else {
                Scalar::Int64((sm(i, 5) % 50) as i64 + 1)
            }
        })
        .collect();
    let idx = Index::from_range(0, n as i64, 1);
    let mut map = BTreeMap::new();
    for (nm, v) in [("k1", &k1), ("k2", &k2), ("a", &a), ("b", &b)] {
        map.insert(nm.to_string(), Column::from_values((*v).clone()).unwrap());
    }
    let df = DataFrame::new_with_column_order(
        idx,
        map,
        vec!["k1".into(), "k2".into(), "a".into(), "b".into()],
    )
    .unwrap();
    let mut f = std::fs::File::create("/tmp/fp_dfgbmk.txt").unwrap();
    let g = |s: u64, m: u64| {
        (0..n)
            .map(move |i| (sm(i, s) % m).to_string())
            .collect::<Vec<_>>()
            .join(",")
    };
    writeln!(f, "K1\t{}", g(1, 6)).unwrap();
    writeln!(f, "K2\t{}", g(2, 5)).unwrap();
    writeln!(
        f,
        "A\t{}",
        a.iter()
            .map(|x| match x {
                Scalar::Float64(v) => format!("{v}"),
                _ => "NA".into(),
            })
            .collect::<Vec<_>>()
            .join("|")
    )
    .unwrap();
    writeln!(
        f,
        "B\t{}",
        b.iter()
            .map(|x| match x {
                Scalar::Int64(v) => v.to_string(),
                _ => "NA".into(),
            })
            .collect::<Vec<_>>()
            .join("|")
    )
    .unwrap();
    for op in [
        "sum", "mean", "max", "min", "count", "var", "std", "prod", "median",
    ] {
        let gb = df.groupby(&["k1", "k2"]).unwrap();
        let r = match op {
            "sum" => gb.sum(),
            "mean" => gb.mean(),
            "max" => gb.max(),
            "min" => gb.min(),
            "count" => gb.count(),
            "var" => gb.var(),
            "std" => gb.std(),
            "prod" => gb.prod(),
            _ => gb.median(),
        }
        .unwrap();
        dumpdf(&mut f, op, &r);
    }
    println!("wrote /tmp/fp_dfgbmk.txt");
}
