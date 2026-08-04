//! Bench for `Column::argsort_with` on a NULLABLE contiguous-Utf8 column (`LazyNullableUtf8`)
//! after adding the typed arm — na-last MSD byte radix perm over present spans, instead of the
//! generic O(n log n) na-last Scalar comparator (which materializes `Vec<Scalar::Utf8>`).
//! This backs Series/DataFrame sort_values/sort_index/argsort on a nullable string key.
//!
//! NEW = col.argsort_with(true) [typed arm]. COLD = materialize + stable na-last comparator.
//! WARM = same over a pre-materialized Vec.
//!
//! Run: cargo run -p fp-columnar --release --example bench_utf8_argsort_null -- 5000000 20

use fp_columnar::{Column, ValidityMask};
use fp_types::{NullKind, Scalar};

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

fn argsort_scalar(vals: &[Scalar]) -> Vec<usize> {
    let mut idx: Vec<usize> = (0..vals.len()).collect();
    idx.sort_by(|&a, &b| na_last(&vals[a], &vals[b])); // stable
    idx
}

fn na_last(a: &Scalar, b: &Scalar) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    match (a, b) {
        (Scalar::Utf8(x), Scalar::Utf8(y)) => x.cmp(y),
        (Scalar::Utf8(_), _) => Ordering::Less,    // present before missing
        (_, Scalar::Utf8(_)) => Ordering::Greater, // missing last
        _ => Ordering::Equal,                      // both missing: stable ⇒ original order
    }
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
    let mut state: u64 = 0x1234_9E37;
    for _ in 0..n {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
        if (state >> 40) % 5 == 0 {
            validity.set(present.len(), false);
            present.push(false);
        } else {
            bytes.extend_from_slice(format!("k{:07}", (state >> 20) % 900000).as_bytes());
            present.push(true);
        }
        offsets.push(bytes.len());
    }
    let col = Column::from_utf8_values_with_validity(bytes.clone(), offsets.clone(), validity);
    let warm_vals = materialize(&bytes, &offsets, &present);

    let mut best_t = u128::MAX;
    for _ in 0..iters {
        let t = std::time::Instant::now();
        let r = col.argsort_with(true);
        best_t = best_t.min(t.elapsed().as_nanos());
        std::hint::black_box(&r);
    }
    let mut best_cold = u128::MAX;
    for _ in 0..iters {
        let t = std::time::Instant::now();
        let vals = materialize(&bytes, &offsets, &present);
        let r = argsort_scalar(&vals);
        best_cold = best_cold.min(t.elapsed().as_nanos());
        std::hint::black_box(&r);
    }
    let mut best_warm = u128::MAX;
    for _ in 0..iters {
        let t = std::time::Instant::now();
        let r = argsort_scalar(&warm_vals);
        best_warm = best_warm.min(t.elapsed().as_nanos());
        std::hint::black_box(&r);
    }
    let got = col.argsort_with(true);
    let want = argsort_scalar(&warm_vals);
    assert_eq!(got, want, "argsort perm must match the stable na-last comparator");
    println!(
        "argsort_with utf8_nullable n={n} NEW={:>7.2}ms COLD={:>7.2}ms(={:.2}x) WARM={:>7.2}ms(={:.2}x)",
        best_t as f64 / 1e6,
        best_cold as f64 / 1e6,
        best_cold as f64 / best_t as f64,
        best_warm as f64 / 1e6,
        best_warm as f64 / best_t as f64,
    );
}
