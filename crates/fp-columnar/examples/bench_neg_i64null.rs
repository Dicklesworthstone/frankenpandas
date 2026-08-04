//! Bench for `Column::neg` on a nullable Int64 column — the typed nullable-Int64
//! neg arm (sibling of the abs fix). Source is a LAZY nullable Int64 column;
//! without the fast path it clones a 32-byte Scalar per cell.
//!
//! A/B via the `FP_NO_NULL_NEG_I64` env gate (set ⇒ Scalar loop).
//!
//! Run: cargo run -p fp-columnar --example bench_neg_i64null --release -- 5000000 30

use fp_columnar::{Column, ValidityMask};
use fp_types::Scalar;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(5_000_000);
    let iters: usize = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(30);

    let data: Vec<i64> = (0..n as i64).map(|i| (i % 2001) - 1000).collect();
    let mut validity = ValidityMask::all_valid(n);
    for i in (0..n).step_by(7) {
        validity.set(i, false);
    }
    let col = Column::from_i64_values_with_validity(data, validity);

    let mut best = u128::MAX;
    let mut checksum: u64 = 0;
    for _ in 0..iters {
        let t = std::time::Instant::now();
        let r = col.neg().unwrap();
        best = best.min(t.elapsed().as_nanos());
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for v in r.values().iter() {
            let bits = match v {
                Scalar::Int64(x) => *x as u64,
                _ => 0xDEAD_BEEF,
            };
            h ^= bits;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
        checksum = checksum.wrapping_add(h);
    }
    let gated = std::env::var("FP_NO_NULL_NEG_I64").is_ok();
    println!(
        "neg_i64null n={n} iters={iters} fast_path={} best={best}ns ({:.3}ms) checksum={checksum}",
        !gated,
        best as f64 / 1e6,
    );
}
