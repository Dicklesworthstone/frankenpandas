//! Bench for `Column::min`/`max` on nullable Int64/Float64 after adding the typed
//! present-fold arms (siblings of the nullable `median`/`quantile` paths) — skips
//! nanmin/nanmax's Scalar-Vec materialization + per-Scalar is_missing/enum-match.
//!
//! NEW = col.min()/max(). CONTROL = nanmin/nanmax over the (cached) values() ⇒
//! conservative lower bound (NEW never builds the Scalar Vec on a fresh column).
//! Results asserted equal. min/max is a PURE fold (no select_nth to dilute), so
//! the per-element enum-dispatch removal is a bigger fraction than in median.
//!
//! Run: cargo run -p fp-columnar --release --example bench_minmax_null -- 5000000 40

use fp_columnar::{Column, ValidityMask};

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(5_000_000);
    let iters: usize = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(40);

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
    let i_null = Column::from_i64_values_with_validity(idata, validity.clone());
    let f_null = Column::from_f64_values_with_validity(fdata, validity);

    // Warm the lazy Scalar-Vec cache so CONTROL measures fold-only (honest lower
    // bound), matching the median/quantile bench methodology.
    let _ = i_null.values();
    let _ = f_null.values();

    for (label, col) in [("i64_nullable", &i_null), ("f64_nullable", &f_null)] {
        for op in ["min", "max"] {
            let new = |c: &Column| if op == "min" { c.min() } else { c.max() };
            let ctl = |c: &Column| {
                if op == "min" {
                    fp_types::nanmin(c.values())
                } else {
                    fp_types::nanmax(c.values())
                }
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
                "minmax {label:>13} {op:>3} n={n} NEW={:>7.3}ms CONTROL={:>7.3}ms speedup={:.3}x",
                best_t as f64 / 1e6,
                best_c as f64 / 1e6,
                best_c as f64 / best_t as f64,
            );
        }
    }
}
