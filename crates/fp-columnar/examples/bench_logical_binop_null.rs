//! Bench for `Column::logical_and/or/xor` on nullable Bool columns after adding the
//! `typed_bool_binary_nullable` arm — reads the packed `&[bool]` backing + validity of
//! both operands, applies the op over the raw slices, and ANDs the validity masks,
//! instead of the generic per-Scalar loop that boxes a Vec<Scalar> + Column::new.
//!
//! NEW = a.logical_and(b). CONTROL = a replica of the old Scalar loop over the (cached)
//! values() ⇒ conservative lower bound (control does NOT pay the lazy materialization).
//!
//! Run: cargo run -p fp-columnar --release --example bench_logical_binop_null -- 5000000 40

use fp_columnar::{Column, ValidityMask};
use fp_types::{DType, NullKind, Scalar};

fn ref_and_col(a: &[Scalar], b: &[Scalar]) -> Column {
    let out: Vec<Scalar> = a
        .iter()
        .zip(b)
        .map(|(x, y)| {
            if x.is_missing() || y.is_missing() {
                Scalar::Null(NullKind::Null)
            } else {
                let xv = match x {
                    Scalar::Bool(v) => *v,
                    _ => x.to_f64().map(|v| v != 0.0).unwrap_or(false),
                };
                let yv = match y {
                    Scalar::Bool(v) => *v,
                    _ => y.to_f64().map(|v| v != 0.0).unwrap_or(false),
                };
                Scalar::Bool(xv && yv)
            }
        })
        .collect();
    Column::new(DType::Bool, out).unwrap()
}

fn build(n: usize, seed: u64) -> Column {
    let mut state = seed;
    let mut next = || {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        state
    };
    let mut validity = ValidityMask::all_valid(n);
    let data: Vec<bool> = (0..n)
        .map(|i| {
            let r = next();
            if r % 5 == 0 {
                validity.set(i, false);
            }
            (r >> 17) & 1 == 1
        })
        .collect();
    Column::from_bool_values_with_validity(data, validity)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let n: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(5_000_000);
    let iters: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(40);

    let a = build(n, 0x1234_9E37_ABCD_0001);
    let b = build(n, 0x9E37_1234_0002_BEEF);

    let av = a.values().to_vec(); // warm the lazy Scalar-Vec cache for CONTROL
    let bv = b.values().to_vec();

    let mut best_t = u128::MAX;
    for _ in 0..iters {
        let t = std::time::Instant::now();
        let r = a.logical_and(&b).unwrap();
        best_t = best_t.min(t.elapsed().as_nanos());
        std::hint::black_box(&r);
    }
    let mut best_c = u128::MAX;
    for _ in 0..iters {
        let t = std::time::Instant::now();
        let r = ref_and_col(&av, &bv);
        best_c = best_c.min(t.elapsed().as_nanos());
        std::hint::black_box(&r);
    }
    let got = a.logical_and(&b).unwrap();
    let want = ref_and_col(&av, &bv);
    let gv = got.values();
    let wv = want.values();
    assert_eq!(gv.len(), wv.len());
    for k in 0..n {
        assert_eq!(
            format!("{:?}", gv.get(k)),
            format!("{:?}", wv.get(k)),
            "slot {k} mismatch"
        );
    }
    println!(
        "logical_and bool_nullable n={n} NEW={:>7.2}ms CONTROL={:>7.2}ms speedup={:.3}x",
        best_t as f64 / 1e6,
        best_c as f64 / 1e6,
        best_c as f64 / best_t as f64,
    );
}
