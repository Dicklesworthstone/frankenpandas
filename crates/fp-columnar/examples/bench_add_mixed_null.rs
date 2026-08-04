//! Bench for MIXED nullable Int64 × nullable Float64 two-column arithmetic
//! (→ Float64) — the generalized `f64_view_naw` fast path in
//! `try_vectorized_binary`'s Float64 arm. Left is LAZY nullable Int64, right is
//! LAZY nullable Float64. Without the fast path the mixed-nullable pair falls to
//! the from_scalars materialization (both operands → Vec<f64> via Scalar
//! iteration) before the kernel.
//!
//! A/B on the SAME binary via the `FP_NO_F64_NULL_VIEW` env gate (set ⇒
//! from_scalars path). Strip the gate before commit.
//!
//! Run: cargo run -p fp-columnar --example bench_add_mixed_null --release -- 5000000 15 add
//! Control: FP_NO_F64_NULL_VIEW=1 cargo run -p fp-columnar --example bench_add_mixed_null --release -- 5000000 15 add

use fp_columnar::{Column, ValidityMask};
use fp_types::Scalar;

fn nullable_validity(n: usize, null_stride: usize) -> ValidityMask {
    let mut validity = ValidityMask::all_valid(n);
    for i in (0..n).step_by(null_stride) {
        validity.set(i, false);
    }
    validity
}

fn build_nullable_i64(n: usize, mult: u64, add: u64, null_stride: usize) -> Column {
    let data: Vec<i64> = (0..n)
        .map(|i| {
            let h = (i as u64).wrapping_mul(mult).wrapping_add(add);
            (h % 100_003) as i64 - 50_001
        })
        .collect();
    Column::from_i64_values_with_validity(data, nullable_validity(n, null_stride))
}

fn build_nullable_f64(n: usize, mult: u64, add: u64, null_stride: usize) -> Column {
    let data: Vec<f64> = (0..n)
        .map(|i| {
            let h = (i as u64).wrapping_mul(mult).wrapping_add(add);
            ((h % 100_003) as f64 - 50_001.0) * 0.5
        })
        .collect();
    Column::from_f64_values_with_validity(data, nullable_validity(n, null_stride))
}

fn digest_f64(col: &Column) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for v in col.values().iter() {
        let bits = match v {
            Scalar::Float64(x) => x.to_bits(),
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
    let iters: usize = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(15);
    let which = a.get(3).map(String::as_str).unwrap_or("add");

    let x = build_nullable_i64(n, 2_654_435_761, 12_345, 7); // nullable Int64
    let y = build_nullable_f64(n, 40_503, 6_789, 11); // nullable Float64

    let mut best = u128::MAX;
    let mut checksum: u64 = 0;
    for _ in 0..iters {
        let t = std::time::Instant::now();
        let r = match which {
            "sub" => x.sub(&y),
            "mul" => x.mul(&y),
            "div" => x.div(&y),
            _ => x.add(&y),
        }
        .expect("mixed nullable arithmetic");
        best = best.min(t.elapsed().as_nanos());
        checksum = checksum.wrapping_add(digest_f64(&r));
    }
    let gated = std::env::var("FP_NO_F64_NULL_VIEW").is_ok();
    println!(
        "add_mixed_null op={which} n={n} iters={iters} fast_path={} best={best}ns ({:.3}ms) checksum={checksum}",
        !gated,
        best as f64 / 1e6,
    );
}
