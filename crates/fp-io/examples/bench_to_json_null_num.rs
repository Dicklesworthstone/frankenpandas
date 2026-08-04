//! to_json(records) over a DataFrame with NULLABLE numeric columns (Float64 + Int64) + a
//! string column. Before JCol::FN/IN, a null-bearing numeric column forced the WHOLE frame
//! onto the serde tree. NEW builds every column typed; CONTROL builds the name column eagerly
//! (→ serde tree) with identical data, isolating the fast-vs-serde delta the FN/IN arms unlock.
//!
//! Run: cargo run -p fp-io --release --example bench_to_json_null_num -- 1000000 6
use std::collections::BTreeMap;

use fp_columnar::{Column, ValidityMask};
use fp_frame::DataFrame;
use fp_index::Index;
use fp_io::JsonOrient;
use fp_types::Scalar;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(1_000_000);
    let iters: usize = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(6);

    let ids: Vec<i64> = (0..n as i64).collect();
    let price: Vec<f64> = (0..n).map(|i| (i % 100000) as f64 / 100.0).collect();
    let mut price_v = ValidityMask::all_valid(n);
    let qty: Vec<i64> = (0..n as i64).map(|i| i % 1000).collect();
    let mut qty_v = ValidityMask::all_valid(n);
    let active: Vec<bool> = (0..n).map(|i| i % 2 == 0).collect();
    let mut active_v = ValidityMask::all_valid(n);
    let mut nb: Vec<u8> = Vec::with_capacity(n * 8);
    let mut no: Vec<usize> = Vec::with_capacity(n + 1);
    no.push(0);
    let mut names_eager: Vec<Scalar> = Vec::with_capacity(n);
    for i in 0..n {
        if i % 7 == 0 {
            price_v.set(i, false);
        }
        if i % 5 == 0 {
            qty_v.set(i, false);
        }
        if i % 3 == 0 {
            active_v.set(i, false);
        }
        let s = format!("item_{}", i % 5000);
        nb.extend_from_slice(s.as_bytes());
        no.push(nb.len());
        names_eager.push(Scalar::Utf8(s));
    }

    let mk = |name_col: Column| -> DataFrame {
        let mut cols: BTreeMap<String, Column> = BTreeMap::new();
        cols.insert("id".to_string(), Column::from_i64_values_owned(ids.clone()));
        cols.insert(
            "price".to_string(),
            Column::from_f64_values_with_validity(price.clone(), price_v.clone()),
        );
        cols.insert(
            "qty".to_string(),
            Column::from_i64_values_with_validity(qty.clone(), qty_v.clone()),
        );
        cols.insert(
            "active".to_string(),
            Column::from_bool_values_with_validity(active.clone(), active_v.clone()),
        );
        cols.insert("name".to_string(), name_col);
        DataFrame::new(Index::new_known_unique_int64_unit_range(0, n), cols).unwrap()
    };
    let frame_fast = mk(Column::from_utf8_contiguous(nb.clone(), no.clone()));
    let frame_general = mk(Column::from_values(names_eager).unwrap());

    let ctl = frame_general.column("name").unwrap();
    assert!(
        ctl.as_utf8_contiguous().is_none() && ctl.as_nullable_utf8_contiguous().is_none(),
        "control name column must force the serde tree"
    );

    let mut best_t = u128::MAX;
    for _ in 0..iters {
        let t = std::time::Instant::now();
        let s = fp_io::write_json_string(&frame_fast, JsonOrient::Records).unwrap();
        best_t = best_t.min(t.elapsed().as_nanos());
        std::hint::black_box(s.len());
    }
    let mut best_c = u128::MAX;
    for _ in 0..iters {
        let t = std::time::Instant::now();
        let s = fp_io::write_json_string(&frame_general, JsonOrient::Records).unwrap();
        best_c = best_c.min(t.elapsed().as_nanos());
        std::hint::black_box(s.len());
    }
    let sf = fp_io::write_json_string(&frame_fast, JsonOrient::Records).unwrap();
    let sg = fp_io::write_json_string(&frame_general, JsonOrient::Records).unwrap();
    assert_eq!(sf, sg, "fast (FN/IN) JSON must byte-match the serde tree");
    println!(
        "to_json(records) nullable-numeric n={n} NEW(fast)={:>7.2}ms CONTROL(serde)={:>7.2}ms speedup={:.3}x",
        best_t as f64 / 1e6,
        best_c as f64 / 1e6,
        best_c as f64 / best_t as f64,
    );
}
