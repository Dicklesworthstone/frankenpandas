use std::collections::BTreeMap;

use fp_columnar::Column;
use fp_frame::{DataFrame, Series};
use fp_index::Index;
use fp_types::{NullKind, Scalar};

fn sm(i: usize, s: u64) -> u64 {
    let mut h = (i as u64).wrapping_add(s).wrapping_mul(0x9E3779B97F4A7C15);
    h = (h ^ (h >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    h ^ (h >> 31)
}

// Replicate SeriesGroupBy::diff generic (Int64 -> Float64) via public API.
fn ref_diff(vals: &[Scalar], keys: &[Scalar], periods: usize) -> Column {
    let n = vals.len();
    let mut order: Vec<String> = Vec::new();
    let mut groups: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (i, k) in keys.iter().enumerate() {
        let kk = format!("{k:?}");
        groups
            .entry(kk.clone())
            .or_insert_with(|| {
                order.push(kk.clone());
                Vec::new()
            })
            .push(i);
    }
    let mut out = vec![Scalar::Null(NullKind::NaN); n];
    for idxs in groups.values() {
        let gv: Vec<Scalar> = idxs.iter().map(|&i| vals[i].clone()).collect();
        for (li, &si) in idxs.iter().enumerate() {
            if li < periods {
                continue;
            }
            let cur = &gv[li];
            let prev = &gv[li - periods];
            if cur.is_missing() || prev.is_missing() {
                continue;
            }
            let (Ok(c), Ok(p)) = (cur.to_f64(), prev.to_f64()) else {
                continue;
            };
            out[si] = Scalar::Float64(c - p);
        }
    }
    Column::from_values(out).unwrap()
}

fn main() {
    let mut fail = 0;
    for (label, nullable) in [("allvalid", false), ("nullable", true)] {
        let n = 5000usize;
        let card = 19usize;
        let keys: Vec<Scalar> = (0..n)
            .map(|i| Scalar::Int64((sm(i, 1) % card as u64) as i64))
            .collect();
        let iv: Vec<Scalar> = (0..n)
            .map(|i| {
                if nullable && sm(i, 7).is_multiple_of(4) {
                    Scalar::Null(NullKind::Null)
                } else {
                    Scalar::Int64((sm(i, 9) % 200) as i64 - 100)
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
        for periods in [1usize, 2, 3, 7] {
            let s = Series::new("iv", idx.clone(), df.column("iv").unwrap().clone()).unwrap();
            let fast = s.groupby(&k).unwrap().diff(periods).unwrap();
            let refc = ref_diff(&iv, &keys, periods);
            let fc = fast.column();
            let dt_ok = fc.dtype() == refc.dtype();
            let fv = fc.values();
            let rv = refc.values();
            let mut diffs = 0;
            for i in 0..n {
                if format!("{:?}", fv[i]) != format!("{:?}", rv[i]) {
                    diffs += 1;
                    if diffs == 1 {
                        println!("  first diff @{i}: fast={:?} ref={:?}", fv[i], rv[i]);
                    }
                }
            }
            if !dt_ok || diffs > 0 {
                fail += 1;
                println!(
                    "FAIL {label} p={periods}: dtype fast={:?} ref={:?} diffs={diffs}/{n}",
                    fc.dtype(),
                    refc.dtype()
                );
            } else {
                println!("OK {label} p={periods} (dtype {:?}, {n} rows)", fc.dtype());
            }
        }
    }
    println!("{}", if fail == 0 { "ALL GOOD" } else { "FAILED" });
    std::process::exit(if fail == 0 { 0 } else { 1 });
}
