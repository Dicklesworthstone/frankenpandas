//! Bench for `Column::nlargest`/`nsmallest` on a nullable Int64/Float64 column —
//! the typed bounded top-k scan (`nkeep_typed_*_nullable`). Source is a LAZY
//! nullable column (`from_*_values_with_validity`); without the fast path,
//! nlargest falls to `sort_values(false)` — the O(n log n) na-last Scalar
//! comparator — then takes the first `n`.
//!
//! Same-binary A/B: TREATMENT = `col.nlargest(n)` (typed scan); CONTROL =
//! `col.sort_values(false)?` then take `[..n]` (exactly the pre-change path).
//! A folded checksum of both results is asserted equal so the win is honest.
//!
//! Run: cargo run -p fp-columnar --example bench_nlargest_null --release -- 5000000 200 20

use fp_columnar::{Column, ValidityMask};
use fp_types::Scalar;

fn fold_i64(vals: &[Scalar]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for v in vals {
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
    let len: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(5_000_000);
    let n: usize = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(200);
    let iters: usize = a.get(3).and_then(|s| s.parse().ok()).unwrap_or(20);

    // Scattered values with ties; every 5th slot missing (~20%), so plenty of
    // present values remain (n << present) to keep the fast path all-valid.
    let idata: Vec<i64> = (0..len)
        .map(|i| {
            let h = (i as u64).wrapping_mul(2_654_435_761).wrapping_add(12_345);
            (h % 100_003) as i64 - 50_001
        })
        .collect();
    let fdata: Vec<f64> = idata.iter().map(|&x| x as f64 * 0.5).collect();
    let mut validity = ValidityMask::all_valid(len);
    for i in (0..len).step_by(5) {
        validity.set(i, false);
    }
    let icol = Column::from_i64_values_with_validity(idata, validity.clone());
    let fcol = Column::from_f64_values_with_validity(fdata, validity);

    for (label, col) in [("i64", &icol), ("f64", &fcol)] {
        // TREATMENT
        let mut best_t = u128::MAX;
        let mut cksum_t: u64 = 0;
        for _ in 0..iters {
            let t = std::time::Instant::now();
            let top = col.nlargest(n).expect("nlargest");
            best_t = best_t.min(t.elapsed().as_nanos());
            cksum_t = cksum_t.wrapping_add(fold_i64(top.values()));
        }
        // CONTROL (pre-change path: full na-last sort + take)
        let mut best_c = u128::MAX;
        let mut cksum_c: u64 = 0;
        for _ in 0..iters {
            let t = std::time::Instant::now();
            let sorted = col.sort_values(false).expect("sort_values");
            let take = n.min(sorted.len());
            let out = sorted.slice(0, take).expect("slice");
            best_c = best_c.min(t.elapsed().as_nanos());
            cksum_c = cksum_c.wrapping_add(fold_i64(out.values()));
        }
        assert_eq!(cksum_t, cksum_c, "{label}: treatment != control checksum");
        println!(
            "nlargest_null {label} len={len} n={n} iters={iters} \
             treatment={:.3}ms control={:.3}ms speedup={:.3}x checksum={cksum_t:#x}",
            best_t as f64 / 1e6,
            best_c as f64 / 1e6,
            best_c as f64 / best_t as f64,
        );
    }
}
