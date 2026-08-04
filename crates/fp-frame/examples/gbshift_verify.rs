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

// Reference: replicate SeriesGroupBy::transform_groups shift EXACTLY via public API.
fn ref_shift(vals: &[Scalar], keys: &[Scalar], periods: i64) -> Column {
    let n = vals.len();
    // first-seen group -> indices in appearance order
    let mut order: Vec<Scalar> = Vec::new();
    let mut groups: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (i, k) in keys.iter().enumerate() {
        let kk = format!("{k:?}");
        groups
            .entry(kk.clone())
            .or_insert_with(|| {
                order.push(k.clone());
                Vec::new()
            })
            .push(i);
    }
    let mut out = vec![Scalar::Null(NullKind::NaN); n];
    for indices in groups.values() {
        let gv: Vec<Scalar> = indices.iter().map(|&i| vals[i].clone()).collect();
        let m = gv.len();
        let mut t = vec![Scalar::Null(NullKind::NaN); m];
        for (idx, slot) in t.iter_mut().enumerate() {
            let src = idx as i64 - periods;
            if src >= 0 && (src as usize) < m {
                *slot = gv[src as usize].clone();
            }
        }
        for (local, &si) in indices.iter().enumerate() {
            out[si] = t[local].clone();
        }
    }
    Column::from_values(out).unwrap()
}

fn main() {
    let mut fail = 0;
    for (label, nullable) in [("allvalid", false), ("nullable", true)] {
        let n = 5000usize;
        let card = 23usize;
        let keys: Vec<Scalar> = (0..n)
            .map(|i| Scalar::Int64((sm(i, 1) % card as u64) as i64))
            .collect();
        let iv: Vec<Scalar> = (0..n)
            .map(|i| {
                if nullable && sm(i, 7).is_multiple_of(4) {
                    Scalar::Null(NullKind::Null)
                } else {
                    Scalar::Int64((sm(i, 9) % 100) as i64 - 50)
                }
            })
            .collect();
        let idx = Index::from_range(0, n as i64, 1);
        let mut map = BTreeMap::new();
        map.insert("k".to_string(), Column::from_values(keys.clone()).unwrap());
        map.insert("iv".to_string(), Column::from_values(iv.clone()).unwrap());
        let df = DataFrame::new_with_column_order(idx.clone(), map, vec!["k".into(), "iv".into()])
            .unwrap();
        let k = Series::new("k", idx.clone(), df.column("k").unwrap().clone()).unwrap();
        for periods in [1i64, 2, 3, -1, -2, 5] {
            let s = Series::new("iv", idx.clone(), df.column("iv").unwrap().clone()).unwrap();
            let fast = s.groupby(&k).unwrap().shift(periods).unwrap();
            let refc = ref_shift(&iv, &keys, periods);
            // compare dtype + values
            let fc = fast.column();
            let fv = fc.values();
            let rv = refc.values();
            let dt_ok = fc.dtype() == refc.dtype();
            let eqv = |a: &Scalar, b: &Scalar| -> bool {
                if a.is_missing() && b.is_missing() {
                    return true;
                } // missing-equivalent (NullKind canonicalized)
                format!("{a:?}") == format!("{b:?}")
            };
            let mut real_diffs = 0;
            let mut kind_diffs = 0;
            for i in 0..n {
                if !eqv(&fv[i], &rv[i]) {
                    real_diffs += 1;
                } else if format!("{:?}", fv[i]) != format!("{:?}", rv[i]) {
                    kind_diffs += 1;
                }
                // also verify validity bit agrees exactly
                let fmiss = fv[i].is_missing();
                let rmiss = rv[i].is_missing();
                if fmiss != rmiss {
                    real_diffs += 1;
                }
            }
            if !dt_ok || real_diffs > 0 {
                fail += 1;
                println!(
                    "FAIL {label} p={periods}: dtype fast={:?} ref={:?} real_diffs={real_diffs} kind_only={kind_diffs}/{n}",
                    fc.dtype(),
                    refc.dtype()
                );
                for i in 0..n {
                    if !eqv(&fv[i], &rv[i]) {
                        println!("  first real diff @{i}: fast={:?} ref={:?}", fv[i], rv[i]);
                        break;
                    }
                }
            } else {
                println!(
                    "OK {label} p={periods} (dtype {:?}, {n} rows; {kind_diffs} missing-slots canonicalized NaN->Null, values+validity identical)",
                    fc.dtype()
                );
            }
        }
    }
    println!("{}", if fail == 0 { "ALL GOOD" } else { "FAILED" });
    std::process::exit(if fail == 0 { 0 } else { 1 });
}
