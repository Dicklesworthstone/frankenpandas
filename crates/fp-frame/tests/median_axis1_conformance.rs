//! No-mock conformance guard for DataFrame.median(axis=1) routed through the
//! typed reduce_rows_func_f64 + QUICKSELECT fast path. Quickselect must equal a
//! per-row full-sort median for every column-count parity (odd/even/1/2) and for
//! duplicate-heavy / negative rows.

use std::collections::BTreeMap;

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::{Index, IndexLabel};
use fp_types::Scalar;

fn sm(i: usize, s: u64) -> u64 {
    let mut h = (i as u64).wrapping_add(s).wrapping_mul(0x9E3779B97F4A7C15);
    h = (h ^ (h >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    h ^= h >> 31;
    h
}

fn oracle_row_median(vals: &[f64]) -> f64 {
    let mut v = vals.to_vec();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let n = v.len();
    if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) / 2.0
    }
}

fn check(n: usize, ncols: usize) {
    // Build columns; keep raw row values for the oracle.
    let mut cols = BTreeMap::new();
    let mut order = Vec::new();
    let mut rows: Vec<Vec<f64>> = vec![Vec::with_capacity(ncols); n];
    for c in 0..ncols {
        let data: Vec<f64> = (0..n)
            .map(|i| (sm(i, c as u64) % 1000) as f64 - 500.0 + (c as f64) * 0.5)
            .collect();
        for (i, &v) in data.iter().enumerate() {
            rows[i].push(v);
        }
        let name = format!("c{c}");
        cols.insert(name.clone(), Column::from_f64_values(data));
        order.push(name);
    }
    let labels: Vec<IndexLabel> = (0..n as i64).map(IndexLabel::Int64).collect();
    let df = DataFrame::new_with_column_order(Index::new(labels), cols, order).unwrap();
    let got = df.median_axis1().unwrap();
    for (i, row) in rows.iter().enumerate() {
        let want = oracle_row_median(row);
        match &got.values()[i] {
            Scalar::Float64(g) => assert_eq!(
                g.to_bits(),
                want.to_bits(),
                "median row {i} ncols={ncols}: got {g} want {want}"
            ),
            other => panic!("median row {i}: {other:?}"),
        }
    }
}

#[test]
fn median_axis1_quickselect_matches_fullsort() {
    for ncols in [1usize, 2, 3, 7, 8, 20, 21] {
        check(2000, ncols);
    }
}
