use std::{collections::BTreeMap, io::Write};

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::Index;
use fp_types::Scalar;
fn sm(i: usize, s: u64) -> u64 {
    let mut h = (i as u64).wrapping_add(s).wrapping_mul(0x9E3779B97F4A7C15);
    h = (h ^ (h >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    h ^ (h >> 31)
}
fn mkdf(labels: &[i64], seed: u64) -> DataFrame {
    let idx = Index::new(
        labels
            .iter()
            .map(|&v| fp_index::IndexLabel::Int64(v))
            .collect(),
    );
    let mut m = BTreeMap::new();
    let mut order = vec![];
    for c in 0..2 {
        let nm = format!("c{c}");
        let col: Vec<Scalar> = (0..labels.len())
            .map(|i| Scalar::Float64((sm(i, c as u64 + seed) % 50) as f64))
            .collect();
        m.insert(nm.clone(), Column::from_values(col).unwrap());
        order.push(nm);
    }
    DataFrame::new_with_column_order(idx, m, order).unwrap()
}
fn main() {
    // disjoint-ish, unsorted, negative labels to stress alignment
    let la: Vec<i64> = (0..2000)
        .map(|i| ((sm(i, 1) % 3000) as i64) - 1000)
        .collect();
    let mut la2 = la.clone();
    la2.sort();
    la2.dedup(); // unique for a clean align (df needs unique-ish)
    let lb: Vec<i64> = (0..2000)
        .map(|i| ((sm(i, 2) % 3000) as i64) - 800)
        .collect();
    let mut lb2 = lb.clone();
    lb2.sort();
    lb2.dedup();
    let a = mkdf(&la2, 1);
    let b = mkdf(&lb2, 2);
    let r = a.add(&b).unwrap();
    let mut f = std::fs::File::create("/tmp/fp_dfarith.txt").unwrap();
    // dump: index label + c0 value per row (in output order)
    let idx = r.index().labels();
    let c0 = r.column("c0").unwrap().values();
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
    let dumpin = |f: &mut std::fs::File, tag: &str, labels: &[i64], seed: u64| {
        let vals: Vec<String> = (0..labels.len())
            .map(|i| (sm(i, seed) % 50).to_string())
            .collect();
        writeln!(
            f,
            "{tag}L\t{}",
            labels
                .iter()
                .map(|v| v.to_string())
                .collect::<Vec<_>>()
                .join(",")
        )
        .unwrap();
        writeln!(f, "{tag}V\t{}", vals.join(",")).unwrap();
    };
    dumpin(&mut f, "A", &la2, 1);
    dumpin(&mut f, "B", &lb2, 2);
    println!("wrote a={} b={} out={}", la2.len(), lb2.len(), idx.len());
}
