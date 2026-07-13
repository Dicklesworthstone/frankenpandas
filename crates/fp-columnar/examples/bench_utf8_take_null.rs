//! Bench for `Column::take_positions` on a NULLABLE contiguous-Utf8 column
//! (`LazyNullableUtf8`) after adding the nullable gather arm — gathers selected byte spans
//! into a contiguous buffer + validity, instead of the generic per-row Scalar clone that
//! materializes `Vec<Scalar::Utf8>` (a heap String per row) then clones a String per output.
//!
//! NEW = col.take_positions(&perm) [typed arm]. COLD = materialize + per-row Scalar clone
//! gather. WARM = same over a pre-materialized Vec (no input materialization), to check for a
//! warm regression.
//!
//! Run: cargo run -p fp-columnar --release --example bench_utf8_take_null -- 5000000 20

use fp_columnar::{Column, ValidityMask};
use fp_types::{DType, NullKind, Scalar};

fn materialize(bytes: &[u8], offsets: &[usize], present: &[bool]) -> Vec<Scalar> {
    offsets
        .windows(2)
        .enumerate()
        .map(|(i, w)| {
            if present[i] {
                Scalar::Utf8(std::str::from_utf8(&bytes[w[0]..w[1]]).unwrap().to_string())
            } else {
                Scalar::Null(NullKind::Null)
            }
        })
        .collect()
}

fn gather_clone(vals: &[Scalar], perm: &[usize]) -> Column {
    let out: Vec<Scalar> = perm.iter().map(|&p| vals[p].clone()).collect();
    Column::new(DType::Utf8, out).unwrap()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let n: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(5_000_000);
    let iters: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(20);

    let mut bytes: Vec<u8> = Vec::with_capacity(n * 10);
    let mut offsets: Vec<usize> = Vec::with_capacity(n + 1);
    offsets.push(0);
    let mut validity = ValidityMask::all_valid(n);
    let mut present: Vec<bool> = Vec::with_capacity(n);
    for i in 0..n {
        if i % 5 == 0 {
            validity.set(i, false);
            present.push(false);
        } else {
            bytes.extend_from_slice(format!("cat_{:06}", i % 1000).as_bytes());
            present.push(true);
        }
        offsets.push(bytes.len());
    }
    let col = Column::from_utf8_values_with_validity(bytes.clone(), offsets.clone(), validity);
    // Scattered permutation (7919 coprime to n) ⇒ realistic sort/reorder gather.
    let perm: Vec<usize> = (0..n).map(|i| (i.wrapping_mul(7919)) % n).collect();
    let warm_vals = materialize(&bytes, &offsets, &present);

    let mut best_t = u128::MAX;
    for _ in 0..iters {
        let t = std::time::Instant::now();
        let r = col.take_positions(&perm);
        best_t = best_t.min(t.elapsed().as_nanos());
        std::hint::black_box(&r);
    }
    let mut best_cold = u128::MAX;
    for _ in 0..iters {
        let t = std::time::Instant::now();
        let vals = materialize(&bytes, &offsets, &present);
        let r = gather_clone(&vals, &perm);
        best_cold = best_cold.min(t.elapsed().as_nanos());
        std::hint::black_box(&r);
    }
    let mut best_warm = u128::MAX;
    for _ in 0..iters {
        let t = std::time::Instant::now();
        let r = gather_clone(&warm_vals, &perm);
        best_warm = best_warm.min(t.elapsed().as_nanos());
        std::hint::black_box(&r);
    }
    let got = col.take_positions(&perm);
    let want = gather_clone(&warm_vals, &perm);
    assert_eq!(got.values(), want.values(), "take mismatch");
    println!(
        "take_positions utf8_nullable n={n} NEW={:>7.2}ms COLD={:>7.2}ms(={:.2}x) WARM={:>7.2}ms(={:.2}x)",
        best_t as f64 / 1e6,
        best_cold as f64 / 1e6,
        best_cold as f64 / best_t as f64,
        best_warm as f64 / 1e6,
        best_warm as f64 / best_t as f64,
    );
}
