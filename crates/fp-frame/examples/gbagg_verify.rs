use std::{collections::BTreeMap, io::Write};

use fp_columnar::Column;
use fp_frame::{DataFrame, Series};
use fp_index::Index;
use fp_types::{NullKind, Scalar};

fn sm(i: usize, s: u64) -> u64 {
    let mut h = (i as u64).wrapping_add(s).wrapping_mul(0x9E3779B97F4A7C15);
    h = (h ^ (h >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    h ^ (h >> 31)
}

fn dump(f: &mut std::fs::File, tag: &str, s: &Series) {
    // sort by label (int64) for stable compare
    let idx = s.index();
    let col = s.column().values();
    let lbls = idx.labels();
    let mut rows: Vec<(i64, String)> = Vec::new();
    for i in 0..s.len() {
        let lbl = match &lbls[i] {
            fp_index::IndexLabel::Int64(v) => *v,
            _ => i as i64,
        };
        let v = match &col[i] {
            Scalar::Null(_) => "NA".to_string(),
            Scalar::Float64(x) => format!("{x:.6}"),
            Scalar::Int64(x) => format!("{x}.000000"),
            o => format!("{o:?}"),
        };
        rows.push((lbl, v));
    }
    rows.sort_by_key(|r| r.0);
    let joined: Vec<String> = rows.iter().map(|(k, v)| format!("{k}={v}")).collect();
    writeln!(f, "{tag}\t{}", joined.join(",")).unwrap();
}

fn main() {
    let n = 6000usize;
    let card = 13usize;
    // make group 4 fully missing in value
    let keys: Vec<Scalar> = (0..n)
        .map(|i| Scalar::Int64((sm(i, 1) % card as u64) as i64))
        .collect();
    let iv: Vec<Scalar> = (0..n)
        .map(|i| {
            let g = sm(i, 1) % card as u64;
            if g == 4 || sm(i, 7).is_multiple_of(4) {
                Scalar::Null(NullKind::Null)
            } else {
                Scalar::Int64((sm(i, 9) % 100) as i64 - 50)
            }
        })
        .collect();
    let idx = Index::from_range(0, n as i64, 1);
    let mut map = BTreeMap::new();
    map.insert("k".into(), Column::from_values(keys.clone()).unwrap());
    map.insert("iv".into(), Column::from_values(iv.clone()).unwrap());
    let df =
        DataFrame::new_with_column_order(idx.clone(), map, vec!["k".into(), "iv".into()]).unwrap();
    let k = Series::new("k", idx.clone(), df.column("k").unwrap().clone()).unwrap();
    let s = Series::new("iv", idx.clone(), df.column("iv").unwrap().clone()).unwrap();
    let mut f = std::fs::File::create("/tmp/fp_gbagg.txt").unwrap();
    // inputs for python
    let ks: Vec<String> = keys
        .iter()
        .map(|x| {
            if let Scalar::Int64(v) = x {
                v.to_string()
            } else {
                "NA".into()
            }
        })
        .collect();
    let vs: Vec<String> = iv
        .iter()
        .map(|x| match x {
            Scalar::Int64(v) => v.to_string(),
            _ => "NA".into(),
        })
        .collect();
    writeln!(f, "K\t{}", ks.join(",")).unwrap();
    writeln!(f, "V\t{}", vs.join(",")).unwrap();
    dump(&mut f, "sum", &s.groupby(&k).unwrap().sum().unwrap());
    dump(&mut f, "mean", &s.groupby(&k).unwrap().mean().unwrap());
    dump(&mut f, "max", &s.groupby(&k).unwrap().max().unwrap());
    dump(&mut f, "min", &s.groupby(&k).unwrap().min().unwrap());
    println!("wrote /tmp/fp_gbagg.txt");
}
