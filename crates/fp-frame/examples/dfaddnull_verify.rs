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
    let n = 2500usize;
    let mk = |seed: u64, nullmod: u64| {
        let mut lbl: Vec<i64> = (0..n)
            .map(|i| ((sm(i, seed) % 2000) as i64) - 600)
            .collect();
        lbl.sort();
        lbl.dedup();
        let idx = Index::new(
            lbl.iter()
                .map(|&v| fp_index::IndexLabel::Int64(v))
                .collect(),
        );
        let vals: Vec<Scalar> = (0..lbl.len())
            .map(|i| {
                if sm(i, 3).is_multiple_of(nullmod) {
                    Scalar::Null(NullKind::Null)
                } else {
                    Scalar::Float64((sm(i, seed + 1) % 50) as f64)
                }
            })
            .collect();
        let mut m = BTreeMap::new();
        m.insert("c0".to_string(), Column::from_values(vals.clone()).unwrap());
        (
            DataFrame::new_with_column_order(idx, m, vec!["c0".into()]).unwrap(),
            lbl,
            vals,
        )
    };
    let (a, al, av) = mk(1, 3);
    let (b, bl, bv) = mk(2, 4);
    let r = a.add(&b).unwrap();
    let idx = r.index().labels();
    let c0 = r.column("c0").unwrap().values();
    let mut f = std::fs::File::create("/tmp/fp_dfaddnull.txt").unwrap();
    let rows: Vec<String> = (0..idx.len())
        .map(|i| {
            let l = match &idx[i] {
                fp_index::IndexLabel::Int64(v) => v.to_string(),
                o => format!("{o:?}"),
            };
            let v = match &c0[i] {
                Scalar::Float64(x) => {
                    if x.is_nan() {
                        "NA".into()
                    } else {
                        format!("{}", *x as i64)
                    }
                }
                Scalar::Null(_) => "NA".into(),
                o => format!("{o:?}"),
            };
            format!("{l}={v}")
        })
        .collect();
    writeln!(f, "ROWS\t{}", rows.join("|")).unwrap();
    let g = |labels: &[i64], vals: &[Scalar]| {
        labels
            .iter()
            .zip(vals)
            .map(|(l, v)| {
                format!(
                    "{}:{}",
                    l,
                    match v {
                        Scalar::Float64(x) => format!("{}", *x as i64),
                        _ => "NA".into(),
                    }
                )
            })
            .collect::<Vec<_>>()
            .join(",")
    };
    writeln!(f, "A\t{}", g(&al, &av)).unwrap();
    writeln!(f, "B\t{}", g(&bl, &bv)).unwrap();
    println!("out={}", idx.len());
}
