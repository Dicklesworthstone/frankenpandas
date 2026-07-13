//! Bench for `Column::argsort_with` on a nullable Int64 column — the na-last
//! radix path in typed_radix_perm. Source is a LAZY nullable Int64 column;
//! without it, sort falls to the O(n log n) Scalar comparator. The checksum folds
//! the returned permutation so treatment (radix) vs control (Scalar) must match.
//!
//! A/B via the `FP_NO_NULL_SORT_I64` env gate (set ⇒ Scalar comparator).
//!
//! Run: cargo run -p fp-columnar --example bench_sort_i64null --release -- 5000000 15 asc

use fp_columnar::{Column, ValidityMask};

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(5_000_000);
    let iters: usize = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(15);
    let ascending = a.get(3).map(String::as_str).unwrap_or("asc") != "desc";

    // Scattered i64 values (with ties) so present-value radix ordering + tie
    // stability are exercised; every 7th slot missing.
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

    let mut best = u128::MAX;
    let mut checksum: u64 = 0;
    for _ in 0..iters {
        let t = std::time::Instant::now();
        let perm = col.argsort_with(ascending);
        best = best.min(t.elapsed().as_nanos());
        // Order-sensitive fold of the permutation.
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for &p in &perm {
            h ^= p as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
        checksum = checksum.wrapping_add(h);
    }
    let gated = std::env::var("FP_NO_NULL_SORT_I64").is_ok();
    println!(
        "sort_i64null asc={ascending} n={n} iters={iters} fast_path={} best={best}ns ({:.3}ms) checksum={checksum}",
        !gated,
        best as f64 / 1e6,
    );
}
