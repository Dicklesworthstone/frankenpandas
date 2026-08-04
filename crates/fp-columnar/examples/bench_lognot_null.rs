//! Bench for `Column::logical_not` on a nullable Int64 column after adding the typed
//! Int64/Float64 arms — builds the Bool result (v == 0) over the raw &[i64] + an
//! explicit validity mask, instead of the generic per-Scalar loop that boxes a
//! Vec<Scalar> + Column::new. (Float64 shares the pattern.)
//!
//! NEW = col.logical_not(). CONTROL = a replica of the old Scalar loop (Vec<Scalar> +
//! Column::new) over the (cached) values() ⇒ conservative lower bound.
//!
//! Run: cargo run -p fp-columnar --release --example bench_lognot_null -- 5000000 40

use fp_columnar::{Column, ValidityMask};
use fp_types::{DType, NullKind, Scalar};

fn ref_lognot_col(vals: &[Scalar]) -> Column {
    let out: Vec<Scalar> = vals
        .iter()
        .map(|v| {
            if v.is_missing() {
                Scalar::Null(NullKind::Null)
            } else {
                let bv = match v {
                    Scalar::Bool(x) => *x,
                    _ => v.to_f64().map(|x| x != 0.0).unwrap_or(false),
                };
                Scalar::Bool(!bv)
            }
        })
        .collect();
    Column::new(DType::Bool, out).unwrap()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let n: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(5_000_000);
    let iters: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(40);

    let data: Vec<i64> = (0..n).map(|i| (i % 4) as i64 - 1).collect(); // mix of 0 and nonzero
    let mut validity = ValidityMask::all_valid(n);
    for i in (0..n).step_by(5) {
        validity.set(i, false);
    }
    let col = Column::from_i64_values_with_validity(data, validity);

    let vals = col.values().to_vec(); // warm the lazy Scalar-Vec cache for CONTROL

    let mut best_t = u128::MAX;
    for _ in 0..iters {
        let t = std::time::Instant::now();
        let r = col.logical_not().unwrap();
        best_t = best_t.min(t.elapsed().as_nanos());
        std::hint::black_box(&r);
    }
    let mut best_c = u128::MAX;
    for _ in 0..iters {
        let t = std::time::Instant::now();
        let r = ref_lognot_col(col.values());
        best_c = best_c.min(t.elapsed().as_nanos());
        std::hint::black_box(&r);
    }
    let got = col.logical_not().unwrap();
    let want = ref_lognot_col(&vals);
    for k in [0usize, 1] {
        assert_eq!(
            format!("{:?}", got.values().get(k)),
            format!("{:?}", want.values().get(k)),
            "slot {k} mismatch"
        );
    }
    println!(
        "logical_not i64_nullable n={n} NEW={:>7.2}ms CONTROL={:>7.2}ms speedup={:.3}x",
        best_t as f64 / 1e6,
        best_c as f64 / 1e6,
        best_c as f64 / best_t as f64,
    );
}
