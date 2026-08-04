//! Bench for `Column::var`/`std`/`sem` on nullable Int64/Float64 after adding the
//! typed two-pass moments over the present subset (skipping nanvar/nanstd/nansem's
//! Vec<Scalar> materialization + collect_finite).
//!
//! NEW = col.var(1)/std(1)/sem(1). CONTROL = nanvar/nanstd/nansem over the
//! (cached) values() ⇒ conservative lower bound. Results asserted equal.
//!
//! Run: cargo run -p fp-columnar --release --example bench_var_std_null -- 5000000 20

use fp_columnar::{Column, ValidityMask};

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(5_000_000);
    let iters: usize = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(20);
    let ddof = 1usize;

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
        for op in ["var", "std", "sem"] {
            let new = |c: &Column| match op {
                "var" => c.var(ddof),
                "std" => c.std(ddof),
                _ => c.sem(ddof),
            };
            let ctl = |c: &Column| match op {
                "var" => fp_types::nanvar(c.values(), ddof),
                "std" => fp_types::nanstd(c.values(), ddof),
                _ => fp_types::nansem(c.values(), ddof),
            };
            let mut best_t = u128::MAX;
            for _ in 0..iters {
                let t = std::time::Instant::now();
                let r = new(col);
                best_t = best_t.min(t.elapsed().as_nanos());
                std::hint::black_box(&r);
            }
            let mut best_c = u128::MAX;
            for _ in 0..iters {
                let t = std::time::Instant::now();
                let r = ctl(col);
                best_c = best_c.min(t.elapsed().as_nanos());
                std::hint::black_box(&r);
            }
            assert_eq!(
                format!("{:?}", new(col)),
                format!("{:?}", ctl(col)),
                "{label} {op}: NEW != control"
            );
            println!(
                "var_std {label:>13} {op:>3} n={n} NEW={:>7.2}ms CONTROL={:>7.2}ms speedup={:.3}x",
                best_t as f64 / 1e6,
                best_c as f64 / 1e6,
                best_c as f64 / best_t as f64,
            );
        }
    }
}
