//! DataFrame::add_scalar (df op scalar) over a 5M nullable Int64 column — the
//! typed nullable-Int64 scalar-op arm in apply_scalar_op_inner. Source is a LAZY
//! nullable Int64 column; without the fast path it maps a 32-byte Scalar per cell
//! via col.values().
//!
//! A/B via the `FP_NO_NULL_I64_SCALAR` env gate (set ⇒ Scalar map path).
//!
//! Run: cargo run -p fp-frame --example bench_addscalar_i64null --release -- 5000000 30
//! Control: FP_NO_NULL_I64_SCALAR=1 cargo run -p fp-frame --example bench_addscalar_i64null --release -- 5000000 30

use std::collections::BTreeMap;

use fp_columnar::{Column, ValidityMask};
use fp_frame::DataFrame;
use fp_index::Index;
use fp_types::Scalar;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(5_000_000);
    let iters: usize = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(30);

    let data: Vec<i64> = (0..n as i64).map(|i| i % 1000).collect();
    let mut validity = ValidityMask::all_valid(n);
    for i in (0..n).step_by(7) {
        validity.set(i, false);
    }
    let col = Column::from_i64_values_with_validity(data, validity);
    let mut cols = BTreeMap::new();
    cols.insert("a".to_string(), col);
    let df = DataFrame::new(Index::from_range(0, n as i64, 1), cols).unwrap();

    let mut best = u128::MAX;
    let mut checksum: u64 = 0;
    for _ in 0..iters {
        let t = std::time::Instant::now();
        let r = df.add_scalar(5.0).unwrap();
        best = best.min(t.elapsed().as_nanos());
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for v in r.column("a").unwrap().values().iter() {
            let bits = match v {
                Scalar::Float64(x) => x.to_bits(),
                _ => 0xDEAD_BEEF,
            };
            h ^= bits;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
        checksum = checksum.wrapping_add(h);
    }
    let gated = std::env::var("FP_NO_NULL_I64_SCALAR").is_ok();
    println!(
        "add_scalar_i64null n={n} iters={iters} fast_path={} best={best}ns ({:.3}ms) checksum={checksum}",
        !gated,
        best as f64 / 1e6,
    );
}
