//! Dump the corr-bench input matrix + FP's DataFrame.corr() for a differential
//! check against live pandas (br-frankenpandas-fgy9g). Writes raw little-endian
//! f64: /tmp/corr_input.bin = m columns each of n rows (col-major), and
//! /tmp/fp_corr.bin = the m*m corr matrix (row-major). Prints "DIMS n m".
//!
//! Run: cargo run -p fp-frame --example corr_parity_dump --release

use std::io::Write;

use fp_frame::DataFrame;
use fp_columnar::Column;
use fp_index::{Index, IndexLabel};
use std::collections::BTreeMap;

fn splitmix_unit(i: usize, c: usize) -> f64 {
    // Identical to perf_profile::build_corr_frame.
    let mut z = (i as u64)
        .wrapping_mul(0x9e37_79b9_7f4a_7c15)
        .wrapping_add((c as u64).wrapping_mul(0xbf58_476d_1ce4_e5b9));
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^= z >> 31;
    let unit = (z >> 11) as f64 / (1u64 << 53) as f64;
    unit.mul_add(2.0, -1.0)
}

fn main() {
    let n: usize = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(1_000_000);
    let m: usize = std::env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(8);

    let labels: Vec<IndexLabel> = (0..n as i64).map(IndexLabel::Int64).collect();
    let index = Index::new(labels);
    let mut columns = BTreeMap::new();
    let mut order = Vec::with_capacity(m);
    let mut input = Vec::<f64>::with_capacity(n * m);
    for c in 0..m {
        let col: Vec<f64> = (0..n).map(|i| splitmix_unit(i, c)).collect();
        input.extend_from_slice(&col); // col-major: c0 rows, c1 rows, ...
        let name = format!("c{c}");
        columns.insert(name.clone(), Column::from_f64_values(col));
        order.push(name);
    }
    let df = DataFrame::new_with_column_order(index, columns, order.clone()).expect("frame");

    let corr = df.corr().expect("corr");
    // corr is m x m; extract row-major by the same column order.
    let mut fp = Vec::<f64>::with_capacity(m * m);
    #[allow(clippy::needless_range_loop)]
    for ri in 0..m {
        let col = corr.column(&order[ri]).expect("corr col");
        let vals = col.values();
        for cj in 0..m {
            fp.push(vals[cj].to_f64().unwrap_or(f64::NAN));
        }
    }

    let _ = input;
    let mut line = String::from("FPCORR");
    for x in &fp {
        line.push(' ');
        line.push_str(&format!("{x:.17e}"));
    }
    let mut out = std::io::stdout().lock();
    writeln!(out, "DIMS {n} {m}").unwrap();
    writeln!(out, "{line}").unwrap();
}
