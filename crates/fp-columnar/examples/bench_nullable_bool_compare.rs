//! Bench: nullable-Bool col-vs-col `binary_comparison`. The all-valid Bool arm needs
//! BOTH `as_bool_slice()` (validity.all()), so a nullable pair fell to the generic
//! per-element Scalar loop over `Vec<Scalar::Bool>`. The new typed arm reads raw
//! `&[bool]` + validity. ORIG = plant a null so the arm declines (materialize path via
//! an all-valid replica proxy is not possible — instead we time the CURRENT typed arm
//! vs a hand-rolled Scalar-materializing reference that mirrors the old generic loop).
//! Run: cargo run -p fp-columnar --release --example bench_nullable_bool_compare -- 5000000 20

use fp_columnar::{Column, ComparisonOp, ValidityMask};
use fp_types::{NullKind, Scalar};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let n: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(5_000_000);
    let iters: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(20);

    let mut state: u64 = 0xB0015EED;
    let mut la = ValidityMask::all_valid(n);
    let mut ra = ValidityMask::all_valid(n);
    let mut ld = vec![false; n];
    let mut rd = vec![false; n];
    for i in 0..n {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
        if (state >> 40) % 3 == 0 {
            la.set(i, false);
        } else {
            ld[i] = (state >> 20) & 1 == 0;
        }
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
        if (state >> 40) % 3 == 0 {
            ra.set(i, false);
        } else {
            rd[i] = (state >> 20) & 1 == 0;
        }
    }
    let lcol = Column::from_bool_values_with_validity(ld.clone(), la.clone());
    let rcol = Column::from_bool_values_with_validity(rd.clone(), ra.clone());

    // Reference = the OLD generic path: materialize Vec<Scalar::Bool>/Null both sides,
    // per-element is_missing + scalar_compare + Scalar::Bool alloc.
    let lsc: Vec<Scalar> = (0..n)
        .map(|i| if la.get(i) { Scalar::Bool(ld[i]) } else { Scalar::Null(NullKind::Null) })
        .collect();
    let rsc: Vec<Scalar> = (0..n)
        .map(|i| if ra.get(i) { Scalar::Bool(rd[i]) } else { Scalar::Null(NullKind::Null) })
        .collect();

    let bool_cmp = |a: bool, b: bool| !a && b; // Lt

    let mut best_new = u128::MAX;
    for _ in 0..iters {
        let t = std::time::Instant::now();
        let out = lcol.binary_comparison(&rcol, ComparisonOp::Lt).unwrap();
        best_new = best_new.min(t.elapsed().as_nanos());
        std::hint::black_box(&out);
    }
    let mut best_ref = u128::MAX;
    for _ in 0..iters {
        let t = std::time::Instant::now();
        let out: Vec<Scalar> = lsc
            .iter()
            .zip(&rsc)
            .map(|(l, r)| match (l, r) {
                (Scalar::Bool(a), Scalar::Bool(b)) => Scalar::Bool(bool_cmp(*a, *b)),
                _ => Scalar::Null(NullKind::Null),
            })
            .collect();
        best_ref = best_ref.min(t.elapsed().as_nanos());
        std::hint::black_box(&out);
    }
    // Correctness spot-check: typed arm vs reference.
    let got = lcol.binary_comparison(&rcol, ComparisonOp::Lt).unwrap();
    let want: Vec<Scalar> = lsc
        .iter()
        .zip(&rsc)
        .map(|(l, r)| match (l, r) {
            (Scalar::Bool(a), Scalar::Bool(b)) => Scalar::Bool(bool_cmp(*a, *b)),
            _ => Scalar::Null(NullKind::Null),
        })
        .collect();
    assert_eq!(got.values(), want, "typed nullable-bool arm must match generic reference");

    println!(
        "nullable_bool_compare(Lt) n={n} NEW={:.2}ms REF(materialize+scalar_compare)={:.2}ms ({:.1}x)",
        best_new as f64 / 1e6,
        best_ref as f64 / 1e6,
        best_ref as f64 / best_new as f64,
    );
}
