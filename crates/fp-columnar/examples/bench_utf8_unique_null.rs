//! Bench for `Column::unique` on a NULLABLE contiguous-Utf8 column (`LazyNullableUtf8`) after
//! adding the nullable arm — dedups present byte spans directly (first-seen), instead of the
//! generic `Key` loop that materializes `Vec<Scalar::Utf8>` (a heap String per row).
//!
//! NEW = col.unique(). CONTROL = faithful COLD generic replica (materialize present strings +
//! FxHashSet<&str> first-seen dedup, inside the timed loop). FP unique DROPS nulls.
//!
//! Run: cargo run -p fp-columnar --release --example bench_utf8_unique_null -- 5000000 20

use fp_columnar::{Column, ValidityMask};
use fp_types::{DType, Scalar};
use rustc_hash::FxHashSet;

fn ref_unique_cold(bytes: &[u8], offsets: &[usize], present: &[bool]) -> Column {
    let n = offsets.len() - 1;
    // (1) materialize present rows into owned Strings.
    let mut owned: Vec<String> = Vec::with_capacity(n);
    for (i, w) in offsets.windows(2).enumerate() {
        if present[i] {
            owned.push(std::str::from_utf8(&bytes[w[0]..w[1]]).unwrap().to_string());
        }
    }
    // (2) first-seen dedup over &str.
    let mut seen: FxHashSet<&str> = FxHashSet::default();
    let mut out: Vec<Scalar> = Vec::new();
    for s in &owned {
        if seen.insert(s.as_str()) {
            out.push(Scalar::Utf8(s.clone()));
        }
    }
    Column::new(DType::Utf8, out).unwrap()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let n: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(5_000_000);
    let iters: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(20);

    // ~1000 categories, every 5th row missing.
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

    let mut best_t = u128::MAX;
    for _ in 0..iters {
        let t = std::time::Instant::now();
        let r = col.unique().unwrap();
        best_t = best_t.min(t.elapsed().as_nanos());
        std::hint::black_box(&r);
    }
    let mut best_c = u128::MAX;
    for _ in 0..iters {
        let t = std::time::Instant::now();
        let r = ref_unique_cold(&bytes, &offsets, &present);
        best_c = best_c.min(t.elapsed().as_nanos());
        std::hint::black_box(&r);
    }
    let got = col.unique().unwrap();
    let want = ref_unique_cold(&bytes, &offsets, &present);
    assert_eq!(got.values(), want.values(), "unique mismatch");
    println!(
        "unique utf8_nullable n={n} distinct={} NEW={:>7.2}ms CONTROL={:>7.2}ms speedup={:.3}x",
        got.len(),
        best_t as f64 / 1e6,
        best_c as f64 / 1e6,
        best_c as f64 / best_t as f64,
    );
}
