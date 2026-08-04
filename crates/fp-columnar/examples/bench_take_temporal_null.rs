//! Bench for `Column::take_positions` on a nullable Datetime64 / Timedelta64 /
//! Bool column — the typed nullable gathers (siblings of the Int64/Float64 ones).
//! Source is a LAZY nullable variant; without the fast path it clones a 32-byte
//! Scalar per gathered row. Scattered positions (take/iloc/sort-reorder shape).
//!
//! A/B via the `FP_NO_NULL_TAKE_TEMPORAL` env gate (set ⇒ Scalar clone path).
//!
//! Run: cargo run -p fp-columnar --example bench_take_temporal_null --release -- 5000000 30 dt64

use fp_columnar::{Column, ValidityMask};
use fp_types::Scalar;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(5_000_000);
    let iters: usize = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(30);
    let dt = a.get(3).map(String::as_str).unwrap_or("dt64");

    let mut validity = ValidityMask::all_valid(n);
    for i in (0..n).step_by(7) {
        validity.set(i, false);
    }
    let col = match dt {
        "td64" => {
            let data: Vec<i64> = (0..n as i64).map(|i| i.wrapping_mul(86_400)).collect();
            Column::from_timedelta64_values_with_validity(data, validity)
        }
        "bool" => {
            let data: Vec<bool> = (0..n).map(|i| i % 3 == 0).collect();
            Column::from_bool_values_with_validity(data, validity)
        }
        _ => {
            let data: Vec<i64> = (0..n as i64).map(|i| 1_600_000_000_000 + i * 1_000).collect();
            Column::from_datetime64_values_with_validity(data, validity)
        }
    };

    let positions: Vec<usize> = (0..n).map(|i| i.wrapping_mul(2_654_435_761) % n).collect();

    let mut best = u128::MAX;
    let mut checksum: u64 = 0;
    for _ in 0..iters {
        let t = std::time::Instant::now();
        let r = col.take_positions(&positions);
        best = best.min(t.elapsed().as_nanos());
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for v in r.values().iter() {
            let bits = match v {
                Scalar::Datetime64(x) | Scalar::Timedelta64(x) => *x as u64,
                Scalar::Bool(b) => u64::from(*b),
                _ => 0xDEAD_BEEF,
            };
            h ^= bits;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
        checksum = checksum.wrapping_add(h);
    }
    let gated = std::env::var("FP_NO_NULL_TAKE_TEMPORAL").is_ok();
    println!(
        "take_temporal_null dt={dt} n={n} iters={iters} fast_path={} best={best}ns ({:.3}ms) checksum={checksum}",
        !gated,
        best as f64 / 1e6,
    );
}
