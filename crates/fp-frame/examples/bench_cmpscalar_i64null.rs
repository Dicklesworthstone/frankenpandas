//! DataFrame::gt_scalar_df (df > scalar) over a 5M Int64 column — the typed Int64
//! arm in compare_scalar_df (f64-promote over raw i64 + validity). Covers both
//! nullable (LazyNullableInt64) and all-valid Int64; without the arm both fell to
//! the per-cell Scalar loop.
//!
//! A/B via the `FP_NO_NULL_CMP_DF_I64` env gate (set ⇒ Scalar loop).
//!
//! Run: cargo run -p fp-frame --example bench_cmpscalar_i64null --release -- 5000000 30 null
//! (arg3: "null" ⇒ nullable, else all-valid)

use std::collections::BTreeMap;

use fp_columnar::{Column, ValidityMask};
use fp_frame::DataFrame;
use fp_index::Index;
use fp_types::Scalar;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(5_000_000);
    let iters: usize = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(30);
    let nullable = a.get(3).map(String::as_str).unwrap_or("null") == "null";

    let data: Vec<i64> = (0..n as i64).map(|i| i % 1000 - 500).collect();
    let col = if nullable {
        let mut validity = ValidityMask::all_valid(n);
        for i in (0..n).step_by(7) {
            validity.set(i, false);
        }
        Column::from_i64_values_with_validity(data, validity)
    } else {
        Column::from_i64_values(data)
    };
    let mut cols = BTreeMap::new();
    cols.insert("a".to_string(), col);
    let df = DataFrame::new(Index::from_range(0, n as i64, 1), cols).unwrap();
    let probe = Scalar::Int64(0);

    let mut best = u128::MAX;
    let mut checksum: u64 = 0;
    for _ in 0..iters {
        let t = std::time::Instant::now();
        let r = df.gt_scalar_df(&probe).unwrap();
        best = best.min(t.elapsed().as_nanos());
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for v in r.column("a").unwrap().values().iter() {
            let bits = match v {
                Scalar::Bool(b) => u64::from(*b),
                _ => 0xDEAD_BEEF,
            };
            h ^= bits;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
        checksum = checksum.wrapping_add(h);
    }
    let gated = std::env::var("FP_NO_NULL_CMP_DF_I64").is_ok();
    println!(
        "cmpscalar_i64 nullable={nullable} n={n} iters={iters} fast_path={} best={best}ns ({:.3}ms) checksum={checksum}",
        !gated,
        best as f64 / 1e6,
    );
}
