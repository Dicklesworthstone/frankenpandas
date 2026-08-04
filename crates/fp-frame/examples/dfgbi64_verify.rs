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
    let col = df.column("a").unwrap().values();
    let mut rows: Vec<(String, String)> = Vec::new();
    for i in 0..idx.len() {
        let lbl = match &lbls[i] {
            fp_index::IndexLabel::Utf8(v) => v.clone(),
            fp_index::IndexLabel::Int64(v) => v.to_string(),
            o => format!("{o:?}"),
        };
        let v = match &col[i] {
            Scalar::Int64(x) => format!("i{x}"),
            Scalar::Float64(x) => format!("f{x:.5}"),
            Scalar::Null(_) => "NA".into(),
            o => format!("{o:?}"),
        };
        rows.push((lbl, v));
    }
    rows.sort();
    let j: Vec<String> = rows.iter().map(|(k, v)| format!("{k}={v}")).collect();
    writeln!(f, "{tag}\t{}", j.join(",")).unwrap();
}
fn main() {
    let n = 6000usize;
    let mut f = std::fs::File::create("/tmp/fp_dfgbi64.txt").unwrap();
    for (tag, utf8, card) in [("bounded", false, 30usize), ("eager", true, 30usize)] {
        let keys: Vec<Scalar> = if utf8 {
            (0..n)
                .map(|i| Scalar::Utf8(format!("cat{}", sm(i, 1) % card as u64)))
                .collect()
        } else {
            (0..n)
                .map(|i| Scalar::Int64((sm(i, 1) % card as u64) as i64))
                .collect()
        };
        // group whose key%card==4 fully missing; plus 25% nulls
        let a: Vec<Scalar> = (0..n)
            .map(|i| {
                let g = sm(i, 1) % card as u64;
                if g == 4 || sm(i, 7).is_multiple_of(4) {
                    Scalar::Null(NullKind::Null)
                } else {
                    Scalar::Int64((sm(i, 9) % 50) as i64 + 1)
                }
            })
            .collect();
        let idx = Index::from_range(0, n as i64, 1);
        let mut map = BTreeMap::new();
        map.insert("k".into(), Column::from_values(keys.clone()).unwrap());
        map.insert("a".into(), Column::from_values(a.clone()).unwrap());
        let df = DataFrame::new_with_column_order(idx, map, vec!["k".into(), "a".into()]).unwrap();
        let ks: Vec<String> = keys
            .iter()
            .map(|x| match x {
                Scalar::Utf8(v) => v.clone(),
                Scalar::Int64(v) => v.to_string(),
                _ => "NA".into(),
            })
            .collect();
        let av: Vec<String> = a
            .iter()
            .map(|x| match x {
                Scalar::Int64(v) => v.to_string(),
                _ => "NA".into(),
            })
            .collect();
        writeln!(f, "K_{tag}\t{}", ks.join(",")).unwrap();
        writeln!(f, "A_{tag}\t{}", av.join(",")).unwrap();
        for op in [
            "sum", "mean", "max", "min", "count", "prod", "var", "std", "median", "first", "last",
        ] {
            let g = df.groupby(&["k"]).unwrap();
            let r = match op {
                "sum" => g.sum(),
                "mean" => g.mean(),
                "max" => g.max(),
                "min" => g.min(),
                "count" => g.count(),
                "prod" => g.prod(),
                "var" => g.var(),
                "std" => g.std(),
                "median" => g.median(),
                "first" => g.first(),
                _ => g.last(),
            }
            .unwrap();
            dumpdf(&mut f, &format!("{op}_{tag}"), &r);
        }
    }
    println!("wrote /tmp/fp_dfgbi64.txt");
}
