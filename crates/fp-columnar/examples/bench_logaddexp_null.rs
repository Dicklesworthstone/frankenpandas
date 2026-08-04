//! Bench for `Column::logaddexp` on nullable columns after routing it through
//! typed_nan_propagate_binary (it had NO typed path before). Folds the kernel off both
//! raw slices via get_present, instead of the generic per-Scalar loop that boxes a
//! Vec<Scalar> output + Column::new. (float_power/logaddexp2/nextafter share the helper.)
//!
//! NEW = a.logaddexp(&b). CONTROL = a replica of the old generic loop (Vec<Scalar> +
//! Column::new) over the (cached) values() of both ⇒ conservative lower bound.
//!
//! Run: cargo run -p fp-columnar --release --example bench_logaddexp_null -- 5000000 30

use fp_columnar::{Column, ValidityMask};
use fp_types::{DType, Scalar};

fn logaddexp_kernel(x: f64, y: f64) -> f64 {
    if x.is_nan() || y.is_nan() {
        f64::NAN
    } else if x == f64::NEG_INFINITY {
        y
    } else if y == f64::NEG_INFINITY {
        x
    } else if x == f64::INFINITY || y == f64::INFINITY {
        f64::INFINITY
    } else if x >= y {
        x + (y - x).exp().ln_1p()
    } else {
        y + (x - y).exp().ln_1p()
    }
}

fn ref_logaddexp_col(a: &[Scalar], b: &[Scalar]) -> Column {
    let out: Vec<Scalar> = a
        .iter()
        .zip(b.iter())
        .map(|(x, y)| {
            if x.is_missing() || y.is_missing() {
                Scalar::Float64(f64::NAN)
            } else {
                Scalar::Float64(logaddexp_kernel(x.to_f64().unwrap(), y.to_f64().unwrap()))
            }
        })
        .collect();
    Column::new(DType::Float64, out).unwrap()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let n: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(5_000_000);
    let iters: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(30);

    let adata: Vec<f64> = (0..n)
        .map(|i| {
            let h = (i as u64).wrapping_mul(2_654_435_761).wrapping_add(999);
            (h % 100_003) as f64 * 0.001 - 50.0
        })
        .collect();
    let bdata: Vec<i64> = (0..n)
        .map(|i| {
            let h = (i as u64).wrapping_mul(40_503).wrapping_add(7);
            (h % 100) as i64 - 50
        })
        .collect();
    let mut va = ValidityMask::all_valid(n);
    let mut vb = ValidityMask::all_valid(n);
    for i in (0..n).step_by(5) {
        va.set(i, false);
    }
    for i in (0..n).step_by(7) {
        vb.set(i, false);
    }
    let a = Column::from_f64_values_with_validity(adata, va);
    let b = Column::from_i64_values_with_validity(bdata, vb);

    let av = a.values().to_vec(); // warm both lazy Scalar-Vec caches for CONTROL
    let bv = b.values().to_vec();

    let mut best_t = u128::MAX;
    for _ in 0..iters {
        let t = std::time::Instant::now();
        let r = a.logaddexp(&b).unwrap();
        best_t = best_t.min(t.elapsed().as_nanos());
        std::hint::black_box(&r);
    }
    let mut best_c = u128::MAX;
    for _ in 0..iters {
        let t = std::time::Instant::now();
        let r = ref_logaddexp_col(a.values(), b.values());
        best_c = best_c.min(t.elapsed().as_nanos());
        std::hint::black_box(&r);
    }
    let got = a.logaddexp(&b).unwrap();
    let want = ref_logaddexp_col(&av, &bv);
    for k in [0usize, 1, 2] {
        assert_eq!(
            format!("{:?}", got.values().get(k)),
            format!("{:?}", want.values().get(k)),
            "slot {k} mismatch"
        );
    }
    println!(
        "logaddexp f64null_x_i64null n={n} NEW={:>7.2}ms CONTROL={:>7.2}ms speedup={:.3}x",
        best_t as f64 / 1e6,
        best_c as f64 / 1e6,
        best_c as f64 / best_t as f64,
    );
}
