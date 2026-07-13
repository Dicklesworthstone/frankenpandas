//! Bench for `Column::median` on a nullable Int64/Float64 column after adding the
//! typed nullable path (collect present into an f64 Vec ⇒ median_from_f64_vec),
//! skipping nanmedian's Vec<Scalar> materialization + per-Scalar to_f64.
//!
//! NEW = col.median(). CONTROL = nanmedian over the (cached) values() ⇒
//! conservative lower bound (the O(n) select dominates both). Results equal.
//!
//! Run: cargo run -p fp-columnar --release --example bench_median_null -- 5000000 15

use fp_columnar::{Column, ValidityMask};

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(5_000_000);
    let iters: usize = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(15);

    let idata: Vec<i64> = (0..n)
        .map(|i| {
            let h = (i as u64).wrapping_mul(2_654_435_761).wrapping_add(999);
            (h % 1_000_003) as i64 - 500_000
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
        let mut best_t = u128::MAX;
        for _ in 0..iters {
            let t = std::time::Instant::now();
            let r = col.median();
            best_t = best_t.min(t.elapsed().as_nanos());
            std::hint::black_box(&r);
        }
        let mut best_c = u128::MAX;
        for _ in 0..iters {
            let t = std::time::Instant::now();
            let r = fp_types::nanmedian(col.values());
            best_c = best_c.min(t.elapsed().as_nanos());
            std::hint::black_box(&r);
        }
        assert_eq!(
            format!("{:?}", col.median()),
            format!("{:?}", fp_types::nanmedian(col.values())),
            "{label}: NEW != control"
        );
        println!(
            "median {label:>13} n={n} NEW={:>7.2}ms CONTROL={:>7.2}ms speedup={:.3}x",
            best_t as f64 / 1e6,
            best_c as f64 / 1e6,
            best_c as f64 / best_t as f64,
        );
    }
}
