//! Bench for `Column::skew`/`kurt` on nullable Int64/Float64 after adding typed
//! present-central-moments paths (skip nanskew/nankurt's Vec<Scalar> materialize +
//! collect_finite).
//!
//! NEW = col.skew()/kurt(). CONTROL = nanskew/nankurt over the (cached) values() ⇒
//! conservative lower bound. Results asserted equal.
//!
//! Run: cargo run -p fp-columnar --release --example bench_skew_kurt_null -- 5000000 20

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
        for op in ["skew", "kurt"] {
            let is_skew = op == "skew";
            let mut best_t = u128::MAX;
            for _ in 0..iters {
                let t = std::time::Instant::now();
                let r = if is_skew { col.skew() } else { col.kurt() };
                best_t = best_t.min(t.elapsed().as_nanos());
                std::hint::black_box(&r);
            }
            let mut best_c = u128::MAX;
            for _ in 0..iters {
                let t = std::time::Instant::now();
                let r = if is_skew {
                    fp_types::nanskew(col.values())
                } else {
                    fp_types::nankurt(col.values())
                };
                best_c = best_c.min(t.elapsed().as_nanos());
                std::hint::black_box(&r);
            }
            let new_r = if is_skew { col.skew() } else { col.kurt() };
            let ctl_r = if is_skew {
                fp_types::nanskew(col.values())
            } else {
                fp_types::nankurt(col.values())
            };
            assert_eq!(
                format!("{new_r:?}"),
                format!("{ctl_r:?}"),
                "{label} {op}: NEW != control"
            );
            println!(
                "skew_kurt {label:>13} {op:>4} n={n} NEW={:>7.2}ms CONTROL={:>7.2}ms speedup={:.3}x",
                best_t as f64 / 1e6,
                best_c as f64 / 1e6,
                best_c as f64 / best_t as f64,
            );
        }
    }
}
