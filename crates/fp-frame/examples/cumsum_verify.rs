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

fn main() {
    let n = 5000usize;
    // leading missing + interior missing
    let iv: Vec<Scalar> = (0..n)
        .map(|i| {
            if i < 3 || sm(i, 7).is_multiple_of(4) {
                Scalar::Null(NullKind::Null)
            } else {
                Scalar::Int64((sm(i, 9) % 100) as i64 - 50)
            }
        })
        .collect();
    let idx = Index::from_range(0, n as i64, 1);
    let mut map = BTreeMap::new();
    map.insert("iv".into(), Column::from_values(iv.clone()).unwrap());
    let df = DataFrame::new_with_column_order(idx.clone(), map, vec!["iv".into()]).unwrap();
    let s = Series::new("iv", idx.clone(), df.column("iv").unwrap().clone()).unwrap();
    let res = s.cumsum().unwrap();
    let col = res.column().values();
    let mut f = std::fs::File::create("/tmp/fp_cumsum.txt").unwrap();
    let vs: Vec<String> = iv
        .iter()
        .map(|x| match x {
            Scalar::Int64(v) => v.to_string(),
            _ => "NA".into(),
        })
        .collect();
    writeln!(f, "V\t{}", vs.join(",")).unwrap();
    let outs: Vec<String> = (0..n)
        .map(|i| match &col[i] {
            Scalar::Null(_) => "NA".into(),
            Scalar::Float64(x) => format!("{x:.4}"),
            Scalar::Int64(x) => format!("{x}.0000"),
            o => format!("{o:?}"),
        })
        .collect();
    writeln!(f, "OUT\t{}", outs.join(",")).unwrap();
    writeln!(f, "DTYPE\t{:?}", res.column().dtype()).unwrap();
    println!("wrote /tmp/fp_cumsum.txt dtype={:?}", res.column().dtype());
}
