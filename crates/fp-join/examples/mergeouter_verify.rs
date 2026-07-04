use std::{collections::BTreeMap, io::Write};

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::Index;
use fp_join::{JoinType, merge_dataframes};
use fp_types::Scalar;
fn sm(i: usize, s: u64) -> u64 {
    let mut h = (i as u64).wrapping_add(s).wrapping_mul(0x9E3779B97F4A7C15);
    h = (h ^ (h >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    h ^ (h >> 31)
}
fn main() {
    let n = 3000usize;
    let lk: Vec<Scalar> = (0..n)
        .map(|i| Scalar::Int64(((sm(i, 1) % 1800) as i64) * 104729))
        .collect();
    let rk: Vec<Scalar> = (0..n)
        .map(|i| Scalar::Int64(((sm(i, 2) % 1800) as i64) * 104729))
        .collect();
    let lv: Vec<Scalar> = (0..n)
        .map(|i| Scalar::Int64((sm(i, 7) % 100) as i64))
        .collect();
    let rv: Vec<Scalar> = (0..n)
        .map(|i| Scalar::Int64((sm(i, 8) % 100) as i64))
        .collect();
    let mkdf = |k: &Vec<Scalar>, v: &Vec<Scalar>, vn: &str| {
        let mut m = BTreeMap::new();
        m.insert("k".to_string(), Column::from_values(k.clone()).unwrap());
        m.insert(vn.to_string(), Column::from_values(v.clone()).unwrap());
        DataFrame::new_with_column_order(
            Index::from_range(0, n as i64, 1),
            m,
            vec!["k".into(), vn.into()],
        )
        .unwrap()
    };
    let l = mkdf(&lk, &lv, "lv");
    let r = mkdf(&rk, &rv, "rv");
    let m = merge_dataframes(&l, &r, "k", JoinType::Outer).unwrap();
    let kk = m.columns.get("k").unwrap().values();
    let lvv = m.columns.get("lv").unwrap().values();
    let rvv = m.columns.get("rv").unwrap().values();
    let g = |s: &Scalar| match s {
        Scalar::Int64(v) => v.to_string(),
        Scalar::Null(_) => "NA".into(),
        o => format!("{o:?}"),
    };
    let rows: Vec<String> = (0..kk.len())
        .map(|i| format!("{},{},{}", g(&kk[i]), g(&lvv[i]), g(&rvv[i])))
        .collect();
    let mut f = std::fs::File::create("/tmp/fp_mergeouter.txt").unwrap();
    writeln!(f, "ROWS\t{}", rows.join("|")).unwrap();
    writeln!(f, "LK\t{}", lk.iter().map(&g).collect::<Vec<_>>().join(",")).unwrap();
    writeln!(f, "RK\t{}", rk.iter().map(&g).collect::<Vec<_>>().join(",")).unwrap();
    writeln!(f, "LV\t{}", lv.iter().map(&g).collect::<Vec<_>>().join(",")).unwrap();
    writeln!(f, "RV\t{}", rv.iter().map(&g).collect::<Vec<_>>().join(",")).unwrap();
    println!("wrote nrows={}", kk.len());
}
