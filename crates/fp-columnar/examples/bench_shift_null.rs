//! Bench for `Column::left_shift` on nullable Int64 columns after wiring it to the
//! typed_i64_both_present_binary helper — folds x<<clamp(y) off both raw &[i64] +
//! reuses the input-present mask, instead of the generic per-Scalar loop that boxes a
//! Vec<Scalar> output + Column::new. (right_shift shares the helper.)
//!
//! NEW = a.left_shift(&b). CONTROL = a replica of the old generic loop (Vec<Scalar> +
//! Column::new) over the (cached) values() of both ⇒ conservative lower bound.
//!
//! Run: cargo run -p fp-columnar --release --example bench_shift_null -- 5000000 30

use fp_columnar::{Column, ValidityMask};
use fp_types::{DType, NullKind, Scalar};

fn ref_shl_col(a: &[Scalar], b: &[Scalar]) -> Column {
    let out: Vec<Scalar> = a
        .iter()
        .zip(b.iter())
        .map(|(x, y)| {
            if x.is_missing() || y.is_missing() {
                Scalar::Null(NullKind::Null)
            } else {
                match (x, y) {
                    (Scalar::Int64(xi), Scalar::Int64(yi)) => {
                        Scalar::Int64(xi.wrapping_shl((*yi).clamp(0, 63) as u32))
                    }
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
    let iters: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(30);

    let adata: Vec<i64> = (0..n)
        .map(|i| (i as u64).wrapping_mul(2_654_435_761).wrapping_add(999) as i64)
        .collect();
    let bdata: Vec<i64> = (0..n).map(|i| (i % 70) as i64).collect();
    let mut va = ValidityMask::all_valid(n);
    let mut vb = ValidityMask::all_valid(n);
    for i in (0..n).step_by(5) {
        va.set(i, false);
    }
    for i in (0..n).step_by(7) {
        vb.set(i, false);
    }
    let a = Column::from_i64_values_with_validity(adata, va);
    let b = Column::from_i64_values_with_validity(bdata, vb);

    let av = a.values().to_vec(); // warm both lazy Scalar-Vec caches for CONTROL
    let bv = b.values().to_vec();

    let mut best_t = u128::MAX;
    for _ in 0..iters {
        let t = std::time::Instant::now();
        let r = a.left_shift(&b).unwrap();
        best_t = best_t.min(t.elapsed().as_nanos());
        std::hint::black_box(&r);
    }
    let mut best_c = u128::MAX;
    for _ in 0..iters {
        let t = std::time::Instant::now();
        let r = ref_shl_col(a.values(), b.values());
        best_c = best_c.min(t.elapsed().as_nanos());
        std::hint::black_box(&r);
    }
    let got = a.left_shift(&b).unwrap();
    let want = ref_shl_col(&av, &bv);
    for k in [0usize, 1, 2] {
        assert_eq!(
            format!("{:?}", got.values().get(k)),
            format!("{:?}", want.values().get(k)),
            "slot {k} mismatch"
        );
    }
    println!(
        "left_shift i64null_x_i64null n={n} NEW={:>7.2}ms CONTROL={:>7.2}ms speedup={:.3}x",
        best_t as f64 / 1e6,
        best_c as f64 / 1e6,
        best_c as f64 / best_t as f64,
    );
}
