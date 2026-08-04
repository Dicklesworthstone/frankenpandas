//! Bench for `Column::hist_counts` after making it collect present values off the
//! raw slice + validity instead of materializing Vec<Scalar> + per-Scalar to_f64.
//!
//! NEW = col.hist_counts(bins). CONTROL = a replica of the old generic collect +
//! bucketing over the (cached) values() ⇒ conservative lower bound (NEW never builds
//! the Scalar Vec on a fresh column). Counts asserted equal.
//!
//! Run: cargo run -p fp-columnar --release --example bench_hist_counts -- 5000000 30

use fp_columnar::{Column, ValidityMask};
use fp_types::Scalar;

fn ref_hist(vals: &[Scalar], bins: usize) -> Vec<usize> {
    if bins == 0 {
        return Vec::new();
    }
    let nums: Vec<f64> = vals
        .iter()
        .filter(|v| !v.is_missing())
        .filter_map(|v| v.to_f64().ok())
        .filter(|f| !f.is_nan())
        .collect();
    if nums.is_empty() {
        return vec![0; bins];
    }
    let (min, max) = nums
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), &x| {
            (lo.min(x), hi.max(x))
        });
    if (max - min).abs() < f64::EPSILON {
        let mut counts = vec![0; bins];
        counts[0] = nums.len();
        return counts;
    }
    let width = (max - min) / bins as f64;
    let mut counts = vec![0usize; bins];
    for x in &nums {
        let mut idx = ((x - min) / width) as usize;
        if idx >= bins {
            idx = bins - 1;
        }
        counts[idx] += 1;
    }
    counts
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(5_000_000);
    let iters: usize = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(30);
    let bins = 50usize;

    let idata: Vec<i64> = (0..n)
        .map(|i| {
            let h = (i as u64).wrapping_mul(2_654_435_761).wrapping_add(999);
            (h % 100_003) as i64 - 50_000
        })
        .collect();
    let fdata: Vec<f64> = idata.iter().map(|&x| x as f64 * 0.5).collect();
    let mut validity = ValidityMask::all_valid(n);
    for i in (0..n).step_by(5) {
        validity.set(i, false);
    }
    let f_all = Column::from_f64_values_owned(fdata.clone());
    let i_all = Column::from_i64_values_owned(idata.clone());
    let f_null = Column::from_f64_values_with_validity(fdata, validity.clone());
    let i_null = Column::from_i64_values_with_validity(idata, validity);

    for (label, col) in [
        ("f64_allvalid", &f_all),
        ("i64_allvalid", &i_all),
        ("f64_nullable", &f_null),
        ("i64_nullable", &i_null),
    ] {
        let vals = col.values().to_vec(); // warm the lazy Scalar-Vec cache for CONTROL

        let mut best_t = u128::MAX;
        for _ in 0..iters {
            let t = std::time::Instant::now();
            let r = col.hist_counts(bins);
            best_t = best_t.min(t.elapsed().as_nanos());
            std::hint::black_box(&r);
        }
        let mut best_c = u128::MAX;
        for _ in 0..iters {
            let t = std::time::Instant::now();
            let r = ref_hist(col.values(), bins);
            best_c = best_c.min(t.elapsed().as_nanos());
            std::hint::black_box(&r);
        }
        assert_eq!(col.hist_counts(bins), ref_hist(&vals, bins), "{label}: NEW != control");
        println!(
            "hist_counts {label:>13} n={n} bins={bins} NEW={:>7.2}ms CONTROL={:>7.2}ms speedup={:.3}x",
            best_t as f64 / 1e6,
            best_c as f64 / 1e6,
            best_c as f64 / best_t as f64,
        );
    }
}
