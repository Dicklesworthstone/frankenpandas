//! Bench for `Column::duplicated_keep` on a NULLABLE contiguous-Utf8 column
//! (`LazyNullableUtf8`) after adding the nullable arm — computes dup flags over present byte
//! spans (+ a shared None bucket for missing), instead of the generic `Key` loop that
//! materializes `Vec<Scalar::Utf8>` (a heap `String` per row).
//!
//! NEW = col.duplicated_keep("first"). CONTROL = faithful COLD generic replica (materialize
//! present strings from the backing + Option<&str> first-policy dedup, inside the timed loop).
//!
//! Run: cargo run -p fp-columnar --release --example bench_utf8_duplicated_null -- 5000000 20

use fp_columnar::{Column, ValidityMask};
use fp_types::Scalar;
use rustc_hash::FxHashSet;

fn ref_duplicated_first_cold(bytes: &[u8], offsets: &[usize], present: &[bool]) -> Vec<bool> {
    let n = offsets.len() - 1;
    // (1) materialize present rows into owned Strings (None for missing).
    let mut owned: Vec<Option<String>> = Vec::with_capacity(n);
    for (i, w) in offsets.windows(2).enumerate() {
        if present[i] {
            owned.push(Some(std::str::from_utf8(&bytes[w[0]..w[1]]).unwrap().to_string()));
        } else {
            owned.push(None);
        }
    }
    // (2) first-policy dedup over Option<&str> (None = shared null bucket).
    let mut seen: FxHashSet<Option<&str>> = FxHashSet::default();
    let mut flags = vec![false; n];
    for (i, o) in owned.iter().enumerate() {
        let k = o.as_deref();
        flags[i] = !seen.insert(k);
    }
    flags
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

    let mut best_t = u128::MAX;
    for _ in 0..iters {
        let t = std::time::Instant::now();
        let r = col.duplicated_keep("first").unwrap();
        best_t = best_t.min(t.elapsed().as_nanos());
        std::hint::black_box(&r);
    }
    let mut best_c = u128::MAX;
    for _ in 0..iters {
        let t = std::time::Instant::now();
        let r = ref_duplicated_first_cold(&bytes, &offsets, &present);
        best_c = best_c.min(t.elapsed().as_nanos());
        std::hint::black_box(&r);
    }
    let got = col.duplicated_keep("first").unwrap();
    let want = ref_duplicated_first_cold(&bytes, &offsets, &present);
    let gv = got.values();
    assert_eq!(gv.len(), want.len());
    for k in 0..n {
        assert_eq!(gv[k], Scalar::Bool(want[k]), "slot {k} mismatch");
    }
    let n_dup = want.iter().filter(|&&b| b).count();
    println!(
        "duplicated utf8_nullable n={n} dups={n_dup} NEW={:>7.2}ms CONTROL={:>7.2}ms speedup={:.3}x",
        best_t as f64 / 1e6,
        best_c as f64 / 1e6,
        best_c as f64 / best_t as f64,
    );
}
