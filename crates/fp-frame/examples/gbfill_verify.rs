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

fn scal_str(s: &Scalar) -> String {
    match s {
        Scalar::Null(_) => "NA".to_string(),
        Scalar::Int64(v) => format!("{v}"),
        Scalar::Utf8(v) => v.clone(),
        Scalar::Bool(b) => format!("{b}"),
        other => format!("{other:?}"),
    }
}

fn main() {
    let n = 4000usize;
    let card = 17usize;
    // keys with a wide-ish but dense range; include some all-null groups by making group 3 fully null in iv
    let keys: Vec<Scalar> = (0..n)
        .map(|i| Scalar::Int64((sm(i, 1) % card as u64) as i64))
        .collect();
    let iv: Vec<Scalar> = (0..n)
        .map(|i| {
            let g = sm(i, 1) % card as u64;
            if g == 3 || sm(i, 7).is_multiple_of(3) {
                Scalar::Null(NullKind::Null)
            } else {
                Scalar::Int64((sm(i, 9) % 50) as i64)
            }
        })
        .collect();
    let sv: Vec<Scalar> = (0..n)
        .map(|i| {
            if sm(i, 11).is_multiple_of(3) {
                Scalar::Null(NullKind::Null)
            } else {
                Scalar::Utf8(format!("s{}", sm(i, 13) % 40))
            }
        })
        .collect();
    let bv: Vec<Scalar> = (0..n)
        .map(|i| {
            if sm(i, 17).is_multiple_of(4) {
                Scalar::Null(NullKind::Null)
            } else {
                Scalar::Bool(sm(i, 19).is_multiple_of(2))
            }
        })
        .collect();

    let idx = Index::from_range(0, n as i64, 1);
    let mut map = BTreeMap::new();
    map.insert("k".to_string(), Column::from_values(keys.clone()).unwrap());
    map.insert("iv".to_string(), Column::from_values(iv.clone()).unwrap());
    map.insert("sv".to_string(), Column::from_values(sv.clone()).unwrap());
    map.insert("bv".to_string(), Column::from_values(bv.clone()).unwrap());
    let df = DataFrame::new_with_column_order(
        idx.clone(),
        map,
        vec!["k".into(), "iv".into(), "sv".into(), "bv".into()],
    )
    .unwrap();

    let k = Series::new("k", idx.clone(), df.column("k").unwrap().clone()).unwrap();
    let mut out = std::fs::File::create("/tmp/fp_gbfill.txt").unwrap();
    // write inputs first so python builds identical frame
    for (nm, col) in [("k", &keys), ("iv", &iv), ("sv", &sv), ("bv", &bv)] {
        let s: Vec<String> = col.iter().map(scal_str).collect();
        writeln!(out, "IN\t{nm}\t{}", s.join(",")).unwrap();
    }
    let cases: Vec<(&str, Series, Option<usize>, bool)> = vec![
        (
            "iv_ffill_none",
            Series::new("iv", idx.clone(), df.column("iv").unwrap().clone()).unwrap(),
            None,
            true,
        ),
        (
            "iv_bfill_none",
            Series::new("iv", idx.clone(), df.column("iv").unwrap().clone()).unwrap(),
            None,
            false,
        ),
        (
            "iv_ffill_l2",
            Series::new("iv", idx.clone(), df.column("iv").unwrap().clone()).unwrap(),
            Some(2),
            true,
        ),
        (
            "iv_bfill_l1",
            Series::new("iv", idx.clone(), df.column("iv").unwrap().clone()).unwrap(),
            Some(1),
            false,
        ),
        (
            "sv_ffill_none",
            Series::new("sv", idx.clone(), df.column("sv").unwrap().clone()).unwrap(),
            None,
            true,
        ),
        (
            "sv_bfill_none",
            Series::new("sv", idx.clone(), df.column("sv").unwrap().clone()).unwrap(),
            None,
            false,
        ),
        (
            "sv_ffill_l3",
            Series::new("sv", idx.clone(), df.column("sv").unwrap().clone()).unwrap(),
            Some(3),
            true,
        ),
        (
            "bv_ffill_none",
            Series::new("bv", idx.clone(), df.column("bv").unwrap().clone()).unwrap(),
            None,
            true,
        ),
        (
            "bv_bfill_l2",
            Series::new("bv", idx.clone(), df.column("bv").unwrap().clone()).unwrap(),
            Some(2),
            false,
        ),
    ];
    for (name, s, lim, fwd) in cases {
        let g = s.groupby(&k).unwrap();
        let res = if fwd {
            g.ffill(lim).unwrap()
        } else {
            g.bfill(lim).unwrap()
        };
        let vals: Vec<String> = res.column().values().iter().map(scal_str).collect();
        writeln!(out, "OUT\t{name}\t{}", vals.join(",")).unwrap();
    }
    println!("wrote /tmp/fp_gbfill.txt");
}
