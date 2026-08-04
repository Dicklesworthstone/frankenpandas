//! Bench for `Column::sin` on a nullable Int64 column after adding nullable arms to
//! typed_float_unary_par (the helper behind 17 ops: log10/log2/trig/cbrt/…). Parallel
//! f(v as f64) over the raw &[i64] instead of the generic per-Scalar loop that boxes a
//! Vec<Scalar> output + Column::new.
//!
//! NEW = col.sin(). CONTROL = a replica of the old Scalar path (Vec<Scalar> +
//! Column::new) over the (cached) values() ⇒ conservative lower bound. Slots asserted
//! equal. (sin is representative of all 17 ops sharing the helper.)
//!
//! Run: cargo run -p fp-columnar --release --example bench_trig_i64null -- 5000000 30

use fp_columnar::{Column, ValidityMask};
use fp_types::{DType, Scalar};

fn ref_sin_col(vals: &[Scalar]) -> Column {
    let out: Vec<Scalar> = vals
        .iter()
        .map(|v| {
            if v.is_missing() {
                Scalar::Float64(f64::NAN)
            } else {
                match v {
                    Scalar::Int64(x) => Scalar::Float64((*x as f64).sin()),
                    Scalar::Float64(x) => Scalar::Float64(x.sin()),
                    _ => Scalar::Float64(f64::NAN),
                }
            }
        })
        .collect();
    Column::new(DType::Float64, out).unwrap()
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(5_000_000);
    let iters: usize = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(30);

    let idata: Vec<i64> = (0..n)
        .map(|i| {
            let h = (i as u64).wrapping_mul(2_654_435_761).wrapping_add(999);
            (h % 100_003) as i64 - 50_000
        })
        .collect();
    let mut validity = ValidityMask::all_valid(n);
    for i in (0..n).step_by(5) {
        validity.set(i, false);
    }
    let i_null = Column::from_i64_values_with_validity(idata, validity);

    let vals = i_null.values().to_vec(); // warm the lazy Scalar-Vec cache for CONTROL

    let mut best_t = u128::MAX;
    for _ in 0..iters {
        let t = std::time::Instant::now();
        let r = i_null.sin().unwrap();
        best_t = best_t.min(t.elapsed().as_nanos());
        std::hint::black_box(&r);
    }
    let mut best_c = u128::MAX;
    for _ in 0..iters {
        let t = std::time::Instant::now();
        let r = ref_sin_col(i_null.values());
        best_c = best_c.min(t.elapsed().as_nanos());
        std::hint::black_box(&r);
    }
    let got = i_null.sin().unwrap();
    let want = ref_sin_col(&vals);
    for k in [0usize, 1] {
        assert_eq!(
            format!("{:?}", got.values().get(k)),
            format!("{:?}", want.values().get(k)),
            "slot {k} mismatch"
        );
    }
    println!(
        "sin i64_nullable n={n} NEW={:>7.2}ms CONTROL={:>7.2}ms speedup={:.3}x",
        best_t as f64 / 1e6,
        best_c as f64 / 1e6,
        best_c as f64 / best_t as f64,
    );
}
