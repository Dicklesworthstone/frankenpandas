//! Bench for `Column::weighted_mean`/`average` after adding the typed pairwise fast
//! path (sibling of cov/corr) — folds both raw slices via get_present instead of
//! materializing BOTH columns' Vec<Scalar> + per-Scalar is_missing/to_f64.
//!
//! NEW = col.weighted_mean(w). CONTROL = a replica of the pre-typed generic loop
//! over the (cached) values() of both columns ⇒ conservative lower bound (NEW never
//! builds EITHER column's Scalar Vec on a fresh pair). Results asserted equal.
//!
//! Run: cargo run -p fp-columnar --release --example bench_weighted_mean -- 5000000 30

use fp_columnar::{Column, ValidityMask};
use fp_types::Scalar;

fn ref_wmean(v: &[Scalar], w: &[Scalar]) -> Scalar {
    let mut sum = 0.0;
    let mut weight_sum = 0.0;
    for (a, b) in v.iter().zip(w.iter()) {
        if a.is_missing() || b.is_missing() {
            continue;
        }
        let vf = match a.to_f64() {
            Ok(x) => x,
            Err(_) => continue,
        };
        let wf = match b.to_f64() {
            Ok(x) => x,
            Err(_) => continue,
        };
        sum += vf * wf;
        weight_sum += wf;
    }
    if weight_sum == 0.0 {
        return Scalar::Null(fp_types::NullKind::NaN);
    }
    Scalar::Float64(sum / weight_sum)
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(5_000_000);
    let iters: usize = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(30);

    let vdata: Vec<f64> = (0..n)
        .map(|i| {
            let h = (i as u64).wrapping_mul(2_654_435_761).wrapping_add(999);
            (h % 100_003) as f64 * 0.5 - 25_000.0
        })
        .collect();
    let widata: Vec<i64> = (0..n)
        .map(|i| {
            let h = (i as u64).wrapping_mul(40_503).wrapping_add(7);
            (h % 97) as i64 + 1
        })
        .collect();
    let wfdata: Vec<f64> = widata.iter().map(|&x| x as f64 * 0.5).collect();
    let mut validity = ValidityMask::all_valid(n);
    for i in (0..n).step_by(5) {
        validity.set(i, false);
    }
    let v_f64 = Column::from_f64_values_with_validity(vdata, validity.clone());
    let w_i64 = Column::from_i64_values_with_validity(widata, validity.clone());
    let w_f64 = Column::from_f64_values_with_validity(wfdata, validity);

    for (label, vcol, wcol) in [
        ("f64_x_i64", &v_f64, &w_i64),
        ("f64_x_f64", &v_f64, &w_f64),
    ] {
        // Warm both lazy Scalar-Vec caches so CONTROL measures loop-only.
        let vv = vcol.values().to_vec();
        let wv = wcol.values().to_vec();

        let mut best_t = u128::MAX;
        for _ in 0..iters {
            let t = std::time::Instant::now();
            let r = vcol.weighted_mean(wcol);
            best_t = best_t.min(t.elapsed().as_nanos());
            std::hint::black_box(&r);
        }
        let mut best_c = u128::MAX;
        for _ in 0..iters {
            let t = std::time::Instant::now();
            let r = ref_wmean(vcol.values(), wcol.values());
            best_c = best_c.min(t.elapsed().as_nanos());
            std::hint::black_box(&r);
        }
        assert_eq!(
            format!("{:?}", vcol.weighted_mean(wcol)),
            format!("{:?}", ref_wmean(&vv, &wv)),
            "{label}: NEW != control"
        );
        println!(
            "weighted_mean {label:>10} n={n} NEW={:>7.2}ms CONTROL={:>7.2}ms speedup={:.3}x",
            best_t as f64 / 1e6,
            best_c as f64 / 1e6,
            best_c as f64 / best_t as f64,
        );
    }
}
