use std::{collections::BTreeMap, io::Write};

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::Index;
use fp_join::{JoinType, merge_dataframes_on};
use fp_types::Scalar;
fn sm(i: usize, s: u64) -> u64 {
    let mut h = (i as u64).wrapping_add(s).wrapping_mul(0x9E3779B97F4A7C15);
    h = (h ^ (h >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    h ^ (h >> 31)
}
fn main() {
    let n = 3000usize;
    let mk = |seed: u64, vn: &str| {
        let k1: Vec<Scalar> = (0..n)
            .map(|i| Scalar::Int64(((sm(i, seed) % 5000) as i64) * 104729 - 40_000_000))
            .collect();
        let k2: Vec<Scalar> = (0..n)
            .map(|i| Scalar::Int64((sm(i, seed + 10) % 20) as i64))
            .collect();
        let v: Vec<Scalar> = (0..n)
            .map(|i| Scalar::Int64((sm(i, 7) % 100) as i64))
            .collect();
        let mut m = BTreeMap::new();
        m.insert("k1".into(), Column::from_values(k1.clone()).unwrap());
        m.insert("k2".into(), Column::from_values(k2.clone()).unwrap());
        m.insert(vn.to_string(), Column::from_values(v.clone()).unwrap());
        (
            DataFrame::new_with_column_order(
                Index::from_range(0, n as i64, 1),
                m,
                vec!["k1".into(), "k2".into(), vn.into()],
            )
            .unwrap(),
            k1,
            k2,
            v,
        )
    };
    let (l, lk1, lk2, lv) = mk(1, "lv");
    let (r, rk1, rk2, rv) = mk(2, "rv");
    let g = |s: &Scalar| match s {
        Scalar::Int64(v) => v.to_string(),
        Scalar::Float64(v) => format!("{}", *v as i64),
        Scalar::Null(_) => "NA".into(),
        o => format!("{o:?}"),
    };
    let mut f = std::fs::File::create("/tmp/fp_mergemk.txt").unwrap();
    for (tag, jt) in [
        ("inner", JoinType::Inner),
        ("left", JoinType::Left),
        ("outer", JoinType::Outer),
    ] {
        let m = merge_dataframes_on(&l, &r, &["k1", "k2"], jt).unwrap();
        let k1 = m.columns.get("k1").unwrap().values();
        let k2 = m.columns.get("k2").unwrap().values();
        let lvv = m.columns.get("lv").unwrap().values();
        let rvv = m.columns.get("rv").unwrap().values();
        let rows: Vec<String> = (0..k1.len())
            .map(|i| format!("{},{},{},{}", g(&k1[i]), g(&k2[i]), g(&lvv[i]), g(&rvv[i])))
            .collect();
        writeln!(f, "{tag}\t{}", rows.join("|")).unwrap();
    }
    let dv = |v: &Vec<Scalar>| v.iter().map(&g).collect::<Vec<_>>().join(",");
    writeln!(f, "LK1\t{}", dv(&lk1)).unwrap();
    writeln!(f, "LK2\t{}", dv(&lk2)).unwrap();
    writeln!(f, "LV\t{}", dv(&lv)).unwrap();
    writeln!(f, "RK1\t{}", dv(&rk1)).unwrap();
    writeln!(f, "RK2\t{}", dv(&rk2)).unwrap();
    writeln!(f, "RV\t{}", dv(&rv)).unwrap();
    println!("wrote");
}
