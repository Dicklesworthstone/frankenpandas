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
            Scalar::Int64(x) => x.to_string(),
            Scalar::Null(_) => "NA".into(),
            o => format!("{o:?}"),
        };
        rows.push((lbl, v));
    }
    rows.sort_by_key(|r| r.0);
    let joined: Vec<String> = rows.iter().map(|(k, v)| format!("{k}={v}")).collect();
    writeln!(f, "{tag}\t{}", joined.join(",")).unwrap();
}
fn main() {
    let n = 9000usize;
    let mut f = std::fs::File::create("/tmp/fp_gbcount.txt").unwrap();
    for (tag, card, stride) in [
        ("bounded", 37usize, 1i64),
        ("sparse", 600usize, 1_000_003i64),
    ] {
        // group 4 fully-missing value, plus ~25% nulls; also include a key whose rows are ALL null
        let keys: Vec<Scalar> = (0..n)
            .map(|i| Scalar::Int64(((sm(i, 1) % card as u64) as i64) * stride))
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
        let df = DataFrame::new_with_column_order(idx.clone(), map, vec!["k".into(), "iv".into()])
            .unwrap();
        let k = Series::new("k", idx.clone(), df.column("k").unwrap().clone()).unwrap();
        let s = Series::new("iv", idx.clone(), df.column("iv").unwrap().clone()).unwrap();
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
        writeln!(f, "K_{tag}\t{}", ks.join(",")).unwrap();
        writeln!(f, "V_{tag}\t{}", vs.join(",")).unwrap();
        dump(
            &mut f,
            &format!("count_{tag}"),
            &s.groupby(&k).unwrap().count().unwrap(),
        );
    }
    println!("wrote /tmp/fp_gbcount.txt");
}
