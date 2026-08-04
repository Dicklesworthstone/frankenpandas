use std::{collections::BTreeMap, io::Write};

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::Index;
use fp_types::Scalar;
fn main() {
    // small: a index [0,1,2,3], b index [2,3,4,5] -> union [0..5], unmatched both sides
    let mkdf = |labels: &[i64], vals: &[f64]| {
        let idx = Index::new(
            labels
                .iter()
                .map(|&v| fp_index::IndexLabel::Int64(v))
                .collect(),
        );
        let mut m = BTreeMap::new();
        m.insert(
            "c0".to_string(),
            Column::from_values(vals.iter().map(|&x| Scalar::Float64(x)).collect()).unwrap(),
        );
        DataFrame::new_with_column_order(idx, m, vec!["c0".into()]).unwrap()
    };
    let a = mkdf(&[0, 1, 2, 3], &[5.0, 5.0, 5.0, 5.0]);
    let b = mkdf(&[2, 3, 4, 5], &[1.0, 9.0, 1.0, 1.0]);
    let r = a.gt(&b).unwrap();
    let idx = r.index().labels();
    let c0 = r.column("c0").unwrap().values();
    let mut f = std::fs::File::create("/tmp/fp_dfcmp.txt").unwrap();
    let rows: Vec<String> = (0..idx.len())
        .map(|i| {
            let l = match &idx[i] {
                fp_index::IndexLabel::Int64(v) => v.to_string(),
                o => format!("{o:?}"),
            };
            let v = match &c0[i] {
                Scalar::Bool(x) => {
                    if *x {
                        "T"
                    } else {
                        "F"
                    }
                }
                Scalar::Null(_) => "NA",
                o => {
                    let _ = o;
                    "?"
                }
            }
            .to_string();
            format!("{l}={v}")
        })
        .collect();
    writeln!(f, "{}", rows.join("|")).unwrap();
    println!("{}", rows.join("|"));
}
