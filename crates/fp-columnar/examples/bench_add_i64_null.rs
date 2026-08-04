//! Bench for nullable Int64 two-column elementwise arithmetic
//! (`Column::add/sub/mul/floordiv` col-vs-col) — the typed nullable fast path in
//! `try_vectorized_binary`'s Int64 arm. Both operands are LAZY nullable Int64
//! (`from_i64_values_with_validity` → `LazyNullableInt64`), the honest baseline:
//! without the fast path the generic arm materializes BOTH columns via
//! `from_scalars` (forcing each `LazyNullableInt64` to build a full Vec<Scalar>)
//! plus a trailing Vec<Scalar> for the nullable result. The fast path reads both
//! raw &[i64] buffers + validity directly and returns a typed nullable result.
//!
//! A/B on the SAME binary via the `FP_NO_I64_NULL_ARITH` env gate (set ⇒ skip the
//! fast path). Strip the gate before commit.
//!
//! Run: cargo run -p fp-columnar --example bench_add_i64_null --release -- 5000000 12 add
//! Control: FP_NO_I64_NULL_ARITH=1 cargo run -p fp-columnar --example bench_add_i64_null --release -- 5000000 12 add

use fp_columnar::{Column, ValidityMask};
use fp_types::Scalar;

fn build_nullable_i64(n: usize, mult: u64, add: u64, null_stride: usize) -> Column {
    let data: Vec<i64> = (0..n)
        .map(|i| {
            let h = (i as u64).wrapping_mul(mult).wrapping_add(add);
            (h % 100_003) as i64 - 50_001
        })
        .collect();
    let mut validity = ValidityMask::all_valid(n);
    for i in (0..n).step_by(null_stride) {
        validity.set(i, false);
    }
    Column::from_i64_values_with_validity(data, validity)
}

fn digest_i64(col: &Column) -> u64 {
    // Fold the materialized scalars into an order-sensitive checksum so the op
    // result can't be optimized away; missing slots fold a fixed marker.
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for v in col.values().iter() {
        let bits = match v {
            Scalar::Int64(x) => *x as u64,
            _ => 0xDEAD_BEEF,
        };
        h ^= bits;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(5_000_000);
    let iters: usize = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(12);
    let which = a.get(3).map(String::as_str).unwrap_or("add");

    let x = build_nullable_i64(n, 2_654_435_761, 12_345, 7);
    let y = build_nullable_i64(n, 40_503, 6_789, 11);

    let mut best = u128::MAX;
    let mut checksum: u64 = 0;
    for _ in 0..iters {
        let t = std::time::Instant::now();
        let r = match which {
            "sub" => x.sub(&y),
            "mul" => x.mul(&y),
            "floordiv" => x.floordiv(&y),
            _ => x.add(&y),
        }
        .expect("nullable Int64 arithmetic");
        best = best.min(t.elapsed().as_nanos());
        checksum = checksum.wrapping_add(digest_i64(&r));
    }
    let gated = std::env::var("FP_NO_I64_NULL_ARITH").is_ok();
    println!(
        "add_i64_null op={which} n={n} iters={iters} fast_path={} best={best}ns ({:.3}ms) checksum={checksum}",
        !gated,
        best as f64 / 1e6,
    );
}
