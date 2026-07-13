//! Bench for `Column::bitwise_not` on a nullable Int64 column after adding the typed
//! Int64 arms — maps !x over the raw &[i64] + reuses the input validity, instead of the
//! generic per-Scalar loop that boxes a Vec<Scalar> + Column::new.
//!
//! NEW = col.bitwise_not(). CONTROL = a replica of the old Scalar loop (Vec<Scalar> +
//! Column::new) over the (cached) values() ⇒ conservative lower bound.
//!
//! Run: cargo run -p fp-columnar --release --example bench_bitnot_null -- 5000000 40

use fp_columnar::{Column, ValidityMask};
use fp_types::{DType, NullKind, Scalar};

fn ref_not_col(vals: &[Scalar]) -> Column {
    let out: Vec<Scalar> = vals
        .iter()
        .map(|v| {
            if v.is_missing() {
                Scalar::Null(NullKind::Null)
            } else {
                match v {
                    Scalar::Int64(x) => Scalar::Int64(!x),
                    _ => Scalar::Null(NullKind::Null),
                }
            }
        })
        .collect();
    Column::new(DType::Int64, out).unwrap()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let n: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(5_000_000);
    let iters: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(40);

    let data: Vec<i64> = (0..n)
        .map(|i| (i as u64).wrapping_mul(2_654_435_761).wrapping_add(999) as i64)
        .collect();
    let mut validity = ValidityMask::all_valid(n);
    for i in (0..n).step_by(5) {
        validity.set(i, false);
    }
    let col = Column::from_i64_values_with_validity(data, validity);

    let vals = col.values().to_vec(); // warm the lazy Scalar-Vec cache for CONTROL

    let mut best_t = u128::MAX;
    for _ in 0..iters {
        let t = std::time::Instant::now();
        let r = col.bitwise_not().unwrap();
        best_t = best_t.min(t.elapsed().as_nanos());
        std::hint::black_box(&r);
    }
    let mut best_c = u128::MAX;
    for _ in 0..iters {
        let t = std::time::Instant::now();
        let r = ref_not_col(col.values());
        best_c = best_c.min(t.elapsed().as_nanos());
        std::hint::black_box(&r);
    }
    let got = col.bitwise_not().unwrap();
    let want = ref_not_col(&vals);
    for k in [0usize, 1] {
        assert_eq!(
            format!("{:?}", got.values().get(k)),
            format!("{:?}", want.values().get(k)),
            "slot {k} mismatch"
        );
    }
    println!(
        "bitwise_not i64_nullable n={n} NEW={:>7.2}ms CONTROL={:>7.2}ms speedup={:.3}x",
        best_t as f64 / 1e6,
        best_c as f64 / 1e6,
        best_c as f64 / best_t as f64,
    );
}
