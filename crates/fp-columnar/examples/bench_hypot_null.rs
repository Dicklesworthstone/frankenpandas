//! Bench for `Column::hypot` on nullable columns after routing nullable input through
//! typed_nan_propagate_binary — folds f(x,y) off both raw slices via get_present,
//! instead of the generic per-Scalar loop that boxes a Vec<Scalar> output + Column::new.
//! (atan2/fmod/copysign share the same helper.)
//!
//! NEW = a.hypot(&b). CONTROL = a replica of the old generic loop (Vec<Scalar> +
//! Column::new) over the (cached) values() of both ⇒ conservative lower bound.
//!
//! Run: cargo run -p fp-columnar --release --example bench_hypot_null -- 5000000 30

use fp_columnar::{Column, ValidityMask};
use fp_types::{DType, Scalar};

fn ref_hypot_col(a: &[Scalar], b: &[Scalar]) -> Column {
    let out: Vec<Scalar> = a
        .iter()
        .zip(b.iter())
        .map(|(x, y)| {
            if x.is_missing() || y.is_missing() {
                Scalar::Float64(f64::NAN)
            } else {
                Scalar::Float64(x.to_f64().unwrap().hypot(y.to_f64().unwrap()))
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
            (h % 100_003) as f64 * 0.5 - 25_000.0
        })
        .collect();
    let bdata: Vec<i64> = (0..n)
        .map(|i| {
            let h = (i as u64).wrapping_mul(40_503).wrapping_add(7);
            (h % 100_003) as i64 - 50_000
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
        let r = a.hypot(&b).unwrap();
        best_t = best_t.min(t.elapsed().as_nanos());
        std::hint::black_box(&r);
    }
    let mut best_c = u128::MAX;
    for _ in 0..iters {
        let t = std::time::Instant::now();
        let r = ref_hypot_col(a.values(), b.values());
        best_c = best_c.min(t.elapsed().as_nanos());
        std::hint::black_box(&r);
    }
    let got = a.hypot(&b).unwrap();
    let want = ref_hypot_col(&av, &bv);
    for k in [0usize, 1, 2] {
        assert_eq!(
            format!("{:?}", got.values().get(k)),
            format!("{:?}", want.values().get(k)),
            "slot {k} mismatch"
        );
    }
    println!(
        "hypot f64null_x_i64null n={n} NEW={:>7.2}ms CONTROL={:>7.2}ms speedup={:.3}x",
        best_t as f64 / 1e6,
        best_c as f64 / 1e6,
        best_c as f64 / best_t as f64,
    );
}
