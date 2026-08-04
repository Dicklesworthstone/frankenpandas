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
    for cn in ["a".to_string(), "b".to_string()] {
        let cn = &cn;
        let col = df.column(cn).unwrap().values();
        let mut rows: Vec<(String, String)> = Vec::new();
        for i in 0..idx.len() {
            let lbl = match &lbls[i] {
                fp_index::IndexLabel::Utf8(v) => v.clone(),
                o => format!("{o:?}"),
            };
            let v = match &col[i] {
                Scalar::Null(_) => "NA".into(),
                Scalar::Float64(x) => format!("{x:.5}"),
                Scalar::Int64(x) => format!("{x}"),
                o => format!("{o:?}"),
            };
            rows.push((lbl, v));
        }
        rows.sort();
        let j: Vec<String> = rows.iter().map(|(k, v)| format!("{k}={v}")).collect();
        writeln!(f, "{tag}::{cn}\t{}", j.join(",")).unwrap();
    }
}
fn main() {
    let n = 6000usize;
    let card = 30usize;
    let keys: Vec<Scalar> = (0..n)
        .map(|i| Scalar::Utf8(format!("cat{}", sm(i, 1) % card as u64)))
        .collect();
    let a: Vec<Scalar> = (0..n)
        .map(|i| {
            let g = sm(i, 1) % card as u64;
            if g == 4 || sm(i, 7).is_multiple_of(4) {
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
                Scalar::Float64((sm(i, 5) % 100) as f64)
            }
        })
        .collect();
    let idx = Index::from_range(0, n as i64, 1);
    let mut map = BTreeMap::new();
    map.insert("k".into(), Column::from_values(keys.clone()).unwrap());
    map.insert("a".into(), Column::from_values(a.clone()).unwrap());
    map.insert("b".into(), Column::from_values(b.clone()).unwrap());
    let df = DataFrame::new_with_column_order(idx, map, vec!["k".into(), "a".into(), "b".into()])
        .unwrap();
    let mut f = std::fs::File::create("/tmp/fp_dfgbnf.txt").unwrap();
    let ks: Vec<String> = keys
        .iter()
        .map(|x| {
            if let Scalar::Utf8(v) = x {
                v.clone()
            } else {
                "NA".into()
            }
        })
        .collect();
    let av: Vec<String> = a
        .iter()
        .map(|x| match x {
            Scalar::Float64(v) => format!("{v}"),
            _ => "NA".into(),
        })
        .collect();
    let bv: Vec<String> = b
        .iter()
        .map(|x| match x {
            Scalar::Float64(v) => format!("{v}"),
            _ => "NA".into(),
        })
        .collect();
    writeln!(f, "K\t{}", ks.join(",")).unwrap();
    writeln!(f, "A\t{}", av.join(",")).unwrap();
    writeln!(f, "B\t{}", bv.join(",")).unwrap();
    dumpdf(&mut f, "sum", &df.groupby(&["k"]).unwrap().sum().unwrap());
    dumpdf(&mut f, "mean", &df.groupby(&["k"]).unwrap().mean().unwrap());
    dumpdf(&mut f, "max", &df.groupby(&["k"]).unwrap().max().unwrap());
    dumpdf(&mut f, "min", &df.groupby(&["k"]).unwrap().min().unwrap());
    dumpdf(
        &mut f,
        "count",
        &df.groupby(&["k"]).unwrap().count().unwrap(),
    );
    dumpdf(&mut f, "std", &df.groupby(&["k"]).unwrap().std().unwrap());
    println!("wrote /tmp/fp_dfgbnf.txt");
}
