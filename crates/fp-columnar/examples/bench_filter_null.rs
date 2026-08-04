//! Bench for `Column::filter_by_mask` on a nullable Int64 / Float64 column with
//! an all-valid Bool mask — the typed nullable gather in filter_by_mask's
//! `as_bool_slice` branch. Source is a LAZY nullable column; without the fast
//! path the nullable source clones a 32-byte Scalar per selected row + rescans
//! in Column::new. The fast path gathers the raw datum + validity by mask.
//!
//! A/B on the SAME binary via the `FP_NO_NULL_FILTER` env gate (set ⇒ Scalar
//! clone path). Strip the gate before commit.
//!
//! Run: cargo run -p fp-columnar --example bench_filter_null --release -- 5000000 30 i64
//! Control: FP_NO_NULL_FILTER=1 cargo run -p fp-columnar --example bench_filter_null --release -- 5000000 30 i64

use fp_columnar::{Column, ValidityMask};
use fp_types::Scalar;

fn nullable_validity(n: usize, null_stride: usize) -> ValidityMask {
    let mut validity = ValidityMask::all_valid(n);
    for i in (0..n).step_by(null_stride) {
        validity.set(i, false);
    }
    validity
}

fn digest(col: &Column) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for v in col.values().iter() {
        let bits = match v {
            Scalar::Int64(x) => *x as u64,
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
    let iters: usize = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(30);
    let dt = a.get(3).map(String::as_str).unwrap_or("i64");

    let col = if dt == "f64" {
        let data: Vec<f64> = (0..n)
            .map(|i| {
                let h = (i as u64).wrapping_mul(2_654_435_761).wrapping_add(12_345);
                ((h % 100_003) as f64 - 50_001.0) * 0.5
            })
            .collect();
        Column::from_f64_values_with_validity(data, nullable_validity(n, 7))
    } else {
        let data: Vec<i64> = (0..n)
            .map(|i| {
                let h = (i as u64).wrapping_mul(2_654_435_761).wrapping_add(12_345);
                (h % 100_003) as i64 - 50_001
            })
            .collect();
        Column::from_i64_values_with_validity(data, nullable_validity(n, 7))
    };

    // ~50% selective all-valid Bool mask (deterministic).
    let mask_bools: Vec<bool> = (0..n).map(|i| (i * 2_654_435_761) % 2 == 0).collect();
    let mask = Column::from_bool_values(mask_bools);

    let mut best = u128::MAX;
    let mut checksum: u64 = 0;
    for _ in 0..iters {
        let t = std::time::Instant::now();
        let r = col.filter_by_mask(&mask).expect("filter");
        best = best.min(t.elapsed().as_nanos());
        checksum = checksum.wrapping_add(digest(&r));
    }
    let gated = std::env::var("FP_NO_NULL_FILTER").is_ok();
    println!(
        "filter_null dt={dt} n={n} iters={iters} fast_path={} best={best}ns ({:.3}ms) checksum={checksum}",
        !gated,
        best as f64 / 1e6,
    );
}
