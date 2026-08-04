//! Bench for `Column::where_cond` on a nullable Int64 self (all-valid Bool cond) after
//! adding the nullable-Int64-self typed arm — selects cond[i] ? self[i] : other over
//! the raw &[i64] + validity, instead of the generic per-Scalar loop that clones
//! Scalars + boxes a Vec<Scalar> + Column::new.
//!
//! NEW = col.where_cond(&cond, &other). CONTROL = a replica of the old Scalar loop
//! (Vec<Scalar> + Column::new) over the (cached) values() ⇒ conservative lower bound.
//!
//! Run: cargo run -p fp-columnar --release --example bench_where_null -- 5000000 30

use fp_columnar::{Column, ValidityMask};
use fp_types::{DType, NullKind, Scalar};

fn ref_where_col(vals: &[Scalar], cond_vals: &[Scalar], other: &Scalar) -> Column {
    let out: Vec<Scalar> = vals
        .iter()
        .zip(cond_vals.iter())
        .map(|(v, c)| match c {
            Scalar::Bool(true) => v.clone(),
            Scalar::Bool(false) => other.clone(),
            _ => Scalar::Null(NullKind::NaN),
        })
        .collect();
    Column::new(DType::Int64, out).unwrap()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let n: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(5_000_000);
    let iters: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(30);
    let other = Scalar::Int64(-99);

    let data: Vec<i64> = (0..n)
        .map(|i| (i as u64).wrapping_mul(2_654_435_761).wrapping_add(999) as i64)
        .collect();
    let cb: Vec<bool> = (0..n).map(|i| i % 3 != 0).collect();
    let cond = Column::from_bool_values(cb);
    let mut validity = ValidityMask::all_valid(n);
    for i in (0..n).step_by(5) {
        validity.set(i, false);
    }
    let col = Column::from_i64_values_with_validity(data, validity);

    let vals = col.values().to_vec(); // warm the lazy Scalar-Vec cache for CONTROL
    let cond_vals = cond.values().to_vec();

    let mut best_t = u128::MAX;
    for _ in 0..iters {
        let t = std::time::Instant::now();
        let r = col.where_cond(&cond, &other).unwrap();
        best_t = best_t.min(t.elapsed().as_nanos());
        std::hint::black_box(&r);
    }
    let mut best_c = u128::MAX;
    for _ in 0..iters {
        let t = std::time::Instant::now();
        let r = ref_where_col(col.values(), cond.values(), &other);
        best_c = best_c.min(t.elapsed().as_nanos());
        std::hint::black_box(&r);
    }
    let got = col.where_cond(&cond, &other).unwrap();
    let want = ref_where_col(&vals, &cond_vals, &other);
    for k in [0usize, 1, 2] {
        assert_eq!(
            format!("{:?}", got.values().get(k)),
            format!("{:?}", want.values().get(k)),
            "slot {k} mismatch"
        );
    }
    println!(
        "where_cond i64_nullable n={n} NEW={:>7.2}ms CONTROL={:>7.2}ms speedup={:.3}x",
        best_t as f64 / 1e6,
        best_c as f64 / 1e6,
        best_c as f64 / best_t as f64,
    );
}
