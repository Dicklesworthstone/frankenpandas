//! Bench for `Column::floor` on a nullable Float64 column after adding nullable arms to
//! typed_float_unary_finite_preserving (the helper behind floor/ceil/trunc). Maps f(x)
//! over the raw &[f64] + from_f64_values, instead of the generic per-Scalar loop that
//! boxes a Vec<Scalar> + Column::new. (ceil/trunc share the helper.)
//!
//! NEW = col.floor(). CONTROL = a replica of the old Scalar loop (Vec<Scalar> +
//! Column::new) over the (cached) values() ⇒ conservative lower bound.
//!
//! Run: cargo run -p fp-columnar --release --example bench_floor_null -- 5000000 40

use fp_columnar::{Column, ValidityMask};
use fp_types::{DType, Scalar};

fn ref_floor_col(vals: &[Scalar]) -> Column {
    let out: Vec<Scalar> = vals
        .iter()
        .map(|v| {
            if v.is_missing() {
                Scalar::Float64(f64::NAN)
            } else {
                match v {
                    Scalar::Int64(x) => Scalar::Float64(*x as f64),
                    Scalar::Float64(x) => Scalar::Float64(x.floor()),
                    _ => Scalar::Float64(f64::NAN),
                }
            }
        })
        .collect();
    Column::new(DType::Float64, out).unwrap()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let n: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(5_000_000);
    let iters: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(40);

    let data: Vec<f64> = (0..n)
        .map(|i| {
            let h = (i as u64).wrapping_mul(2_654_435_761).wrapping_add(999);
            (h % 100_003) as f64 * 0.371 - 18_000.0
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
        let r = col.floor().unwrap();
        best_t = best_t.min(t.elapsed().as_nanos());
        std::hint::black_box(&r);
    }
    let mut best_c = u128::MAX;
    for _ in 0..iters {
        let t = std::time::Instant::now();
        let r = ref_floor_col(col.values());
        best_c = best_c.min(t.elapsed().as_nanos());
        std::hint::black_box(&r);
    }
    let got = col.floor().unwrap();
    let want = ref_floor_col(&vals);
    for k in [0usize, 1] {
        assert_eq!(
            format!("{:?}", got.values().get(k)),
            format!("{:?}", want.values().get(k)),
            "slot {k} mismatch"
        );
    }
    println!(
        "floor f64_nullable n={n} NEW={:>7.2}ms CONTROL={:>7.2}ms speedup={:.3}x",
        best_t as f64 / 1e6,
        best_c as f64 / 1e6,
        best_c as f64 / best_t as f64,
    );
}
