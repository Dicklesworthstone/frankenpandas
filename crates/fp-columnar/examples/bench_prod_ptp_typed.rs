//! Bench for `Column::prod`/`ptp` after adding typed Int64 + nullable Int64/Float64
//! paths (fold the raw slice + validity, skipping nanprod/nanptp's Vec<Scalar>
//! materialization + per-Scalar is_missing/to_f64).
//!
//! NEW = col.prod()/ptp(). CONTROL = nanprod/nanptp over the (cached) values() ⇒
//! conservative lower bound. Results asserted equal.
//!
//! Run: cargo run -p fp-columnar --release --example bench_prod_ptp_typed -- 5000000 20

use fp_columnar::{Column, ValidityMask};

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(5_000_000);
    let iters: usize = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(20);

    // Values near ±1 so the product stays finite.
    let idata: Vec<i64> = (0..n)
        .map(|i| {
            let h = (i as u64).wrapping_mul(2_654_435_761).wrapping_add(999);
            (h % 3) as i64 - 1
        })
        .collect();
    let fdata: Vec<f64> = (0..n)
        .map(|i| {
            let h = (i as u64).wrapping_mul(0x9E37_79B9);
            0.5 + (h % 3) as f64 * 0.5
        })
        .collect();
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
        for op in ["prod", "ptp"] {
            let is_prod = op == "prod";
            let mut best_t = u128::MAX;
            for _ in 0..iters {
                let t = std::time::Instant::now();
                let r = if is_prod { col.prod() } else { col.ptp() };
                best_t = best_t.min(t.elapsed().as_nanos());
                std::hint::black_box(&r);
            }
            let mut best_c = u128::MAX;
            for _ in 0..iters {
                let t = std::time::Instant::now();
                let r = if is_prod {
                    fp_types::nanprod(col.values())
                } else {
                    fp_types::nanptp(col.values())
                };
                best_c = best_c.min(t.elapsed().as_nanos());
                std::hint::black_box(&r);
            }
            let new_r = if is_prod { col.prod() } else { col.ptp() };
            let ctl_r = if is_prod {
                fp_types::nanprod(col.values())
            } else {
                fp_types::nanptp(col.values())
            };
            assert_eq!(new_r, ctl_r, "{label} {op}: NEW != control");
            println!(
                "prod_ptp {label:>13} {op:>4} n={n} NEW={:>7.2}ms CONTROL={:>7.2}ms speedup={:.3}x",
                best_t as f64 / 1e6,
                best_c as f64 / 1e6,
                best_c as f64 / best_t as f64,
            );
        }
    }
}
