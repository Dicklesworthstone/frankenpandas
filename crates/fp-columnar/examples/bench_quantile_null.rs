//! Bench for `Column::quantile` on nullable Int64/Float64 after adding the typed
//! present-collect arm (sibling of the nullable `median` path) — skips
//! nanquantile's Vec<Scalar> materialization + collect_finite's per-Scalar to_f64.
//!
//! NEW = col.quantile(q). CONTROL = nanquantile over the (cached) values() ⇒
//! conservative lower bound (NEW never builds the Scalar Vec on a fresh column).
//! Results asserted equal. Modest by design: the O(n) select_nth dominates BOTH
//! paths (like median), so the win is only the skipped materialization + to_f64.
//!
//! Run: cargo run -p fp-columnar --release --example bench_quantile_null -- 5000000 20

use fp_columnar::{Column, ValidityMask};

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(5_000_000);
    let iters: usize = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(20);

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
    let i_all = Column::from_i64_values_owned(idata.clone());
    let i_null = Column::from_i64_values_with_validity(idata, validity.clone());
    let f_null = Column::from_f64_values_with_validity(fdata, validity);

    for (label, col) in [
        ("i64_allvalid", &i_all),
        ("i64_nullable", &i_null),
        ("f64_nullable", &f_null),
    ] {
        for q in [0.25f64, 0.5, 0.9] {
            let mut best_t = u128::MAX;
            for _ in 0..iters {
                let t = std::time::Instant::now();
                let r = col.quantile(q);
                best_t = best_t.min(t.elapsed().as_nanos());
                std::hint::black_box(&r);
            }
            let mut best_c = u128::MAX;
            for _ in 0..iters {
                let t = std::time::Instant::now();
                let r = fp_types::nanquantile(col.values(), q);
                best_c = best_c.min(t.elapsed().as_nanos());
                std::hint::black_box(&r);
            }
            assert_eq!(
                format!("{:?}", col.quantile(q)),
                format!("{:?}", fp_types::nanquantile(col.values(), q)),
                "{label} q={q}: NEW != control"
            );
            println!(
                "quantile {label:>13} q={q:>4} n={n} NEW={:>7.2}ms CONTROL={:>7.2}ms speedup={:.3}x",
                best_t as f64 / 1e6,
                best_c as f64 / 1e6,
                best_c as f64 / best_t as f64,
            );
        }
    }
}
