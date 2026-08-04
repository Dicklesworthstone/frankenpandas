//! Bench for `Column::take_positions` on a nullable Int64 column — the typed
//! nullable-Int64 gather (sibling of the existing typed_f64 gather). Source is a
//! LAZY `LazyNullableInt64` (from_i64_values_with_validity); without the fast
//! path the nullable source clones a 32-byte Scalar per gathered row. Positions
//! are a scattered set (take/iloc/sort-reorder shape).
//!
//! A/B on the SAME binary via the `FP_NO_NULL_TAKE_I64` env gate (set ⇒ Scalar
//! clone path). Strip the gate before commit.
//!
//! Run: cargo run -p fp-columnar --example bench_take_i64_null --release -- 5000000 30
//! Control: FP_NO_NULL_TAKE_I64=1 cargo run -p fp-columnar --example bench_take_i64_null --release -- 5000000 30

use fp_columnar::{Column, ValidityMask};
use fp_types::Scalar;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(5_000_000);
    let iters: usize = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(30);

    let data: Vec<i64> = (0..n)
        .map(|i| {
            let h = (i as u64).wrapping_mul(2_654_435_761).wrapping_add(12_345);
            (h % 100_003) as i64 - 50_001
        })
        .collect();
    let mut validity = ValidityMask::all_valid(n);
    for i in (0..n).step_by(7) {
        validity.set(i, false);
    }
    let col = Column::from_i64_values_with_validity(data, validity);

    // Scattered positions (n reads, with repeats — a take/iloc/reorder shape).
    let positions: Vec<usize> = (0..n)
        .map(|i| (i.wrapping_mul(2_654_435_761)) % n)
        .collect();

    let mut best = u128::MAX;
    let mut checksum: u64 = 0;
    for _ in 0..iters {
        let t = std::time::Instant::now();
        let r = col.take_positions(&positions);
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
    let gated = std::env::var("FP_NO_NULL_TAKE_I64").is_ok();
    println!(
        "take_i64_null n={n} iters={iters} fast_path={} best={best}ns ({:.3}ms) checksum={checksum}",
        !gated,
        best as f64 / 1e6,
    );
}
