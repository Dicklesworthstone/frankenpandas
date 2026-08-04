//! Bench for `Column::heaviside` on a nullable Float64 column after wiring it to the
//! typed_float_unary helper (capturing h0) — maps the kernel off the raw &[f64] +
//! from_f64_values, instead of the generic per-Scalar loop that boxes a Vec<Scalar>
//! output + Column::new. (ldexp shares the pattern.)
//!
//! NEW = col.heaviside(h0). CONTROL = a replica of the old Scalar loop (Vec<Scalar> +
//! Column::new) over the (cached) values() ⇒ conservative lower bound.
//!
//! Run: cargo run -p fp-columnar --release --example bench_heaviside_null -- 5000000 30

use fp_columnar::{Column, ValidityMask};
use fp_types::{DType, Scalar};

fn ref_heaviside_col(vals: &[Scalar], h0: f64) -> Column {
    let out: Vec<Scalar> = vals
        .iter()
        .map(|v| {
            if v.is_missing() {
                return Scalar::Float64(f64::NAN);
            }
            match v {
                Scalar::Float64(x) => Scalar::Float64(if x.is_nan() {
                    f64::NAN
                } else if *x < 0.0 {
                    0.0
                } else if *x > 0.0 {
                    1.0
                } else {
                    h0
                }),
                Scalar::Int64(x) => Scalar::Float64(if *x < 0 {
                    0.0
                } else if *x > 0 {
                    1.0
                } else {
                    h0
                }),
                _ => Scalar::Float64(f64::NAN),
            }
        })
        .collect();
    Column::new(DType::Float64, out).unwrap()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let n: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(5_000_000);
    let iters: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(30);
    let h0 = 0.5f64;

    let data: Vec<f64> = (0..n)
        .map(|i| {
            let h = (i as u64).wrapping_mul(2_654_435_761).wrapping_add(999);
            (h % 100_003) as f64 * 0.5 - 25_000.0
        })
        .collect();
    let mut validity = ValidityMask::all_valid(n);
    for i in (0..n).step_by(5) {
        validity.set(i, false);
    }
    let col = Column::from_f64_values_with_validity(data, validity);

    let vals = col.values().to_vec(); // warm the lazy Scalar-Vec cache for CONTROL

    let mut best_t = u128::MAX;
    for _ in 0..iters {
        let t = std::time::Instant::now();
        let r = col.heaviside(h0).unwrap();
        best_t = best_t.min(t.elapsed().as_nanos());
        std::hint::black_box(&r);
    }
    let mut best_c = u128::MAX;
    for _ in 0..iters {
        let t = std::time::Instant::now();
        let r = ref_heaviside_col(col.values(), h0);
        best_c = best_c.min(t.elapsed().as_nanos());
        std::hint::black_box(&r);
    }
    let got = col.heaviside(h0).unwrap();
    let want = ref_heaviside_col(&vals, h0);
    for k in [0usize, 1] {
        assert_eq!(
            format!("{:?}", got.values().get(k)),
            format!("{:?}", want.values().get(k)),
            "slot {k} mismatch"
        );
    }
    println!(
        "heaviside f64_nullable n={n} NEW={:>7.2}ms CONTROL={:>7.2}ms speedup={:.3}x",
        best_t as f64 / 1e6,
        best_c as f64 / 1e6,
        best_c as f64 / best_t as f64,
    );
}
