//! Bench for `Column::mode` on a NULLABLE contiguous-Utf8 column (`LazyNullableUtf8`) after
//! adding the nullable arm — tallies present byte spans directly, instead of the generic
//! `Key` tally that materializes `Vec<Scalar::Utf8>` (a heap String per row).
//!
//! NEW = col.mode(). CONTROL = faithful COLD generic replica (materialize present strings +
//! FxHashMap<&str> tally + max-count winners, inside the timed loop).
//!
//! Run: cargo run -p fp-columnar --release --example bench_utf8_mode_null -- 5000000 20

use fp_columnar::{Column, ValidityMask};
use fp_types::{DType, Scalar};
use rustc_hash::FxHashMap;

fn ref_mode_cold(bytes: &[u8], offsets: &[usize], present: &[bool]) -> Column {
    let n = offsets.len() - 1;
    // (1) materialize present rows into owned Strings.
    let mut owned: Vec<String> = Vec::with_capacity(n);
    for (i, w) in offsets.windows(2).enumerate() {
        if present[i] {
            owned.push(std::str::from_utf8(&bytes[w[0]..w[1]]).unwrap().to_string());
        }
    }
    // (2) tally + max-count winners, sorted ascending.
    let mut counts: FxHashMap<&str, usize> = FxHashMap::default();
    for s in &owned {
        *counts.entry(s.as_str()).or_insert(0) += 1;
    }
    if counts.is_empty() {
        return Column::new(DType::Utf8, Vec::new()).unwrap();
    }
    let max_count = counts.values().copied().max().unwrap_or(0);
    let mut winners: Vec<&str> = counts
        .iter()
        .filter_map(|(s, c)| if *c == max_count { Some(*s) } else { None })
        .collect();
    winners.sort_unstable();
    let out: Vec<Scalar> = winners.iter().map(|s| Scalar::Utf8((*s).to_string())).collect();
    Column::new(DType::Utf8, out).unwrap()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let n: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(5_000_000);
    let iters: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(20);

    // ~997 categories, but every 3rd present row is "cat_000042" ⇒ a clear single mode.
    let mut bytes: Vec<u8> = Vec::with_capacity(n * 10);
    let mut offsets: Vec<usize> = Vec::with_capacity(n + 1);
    offsets.push(0);
    let mut validity = ValidityMask::all_valid(n);
    let mut present: Vec<bool> = Vec::with_capacity(n);
    for i in 0..n {
        if i % 5 == 0 {
            validity.set(i, false);
            present.push(false);
        } else if i % 3 == 0 {
            bytes.extend_from_slice(b"cat_000042");
            present.push(true);
        } else {
            bytes.extend_from_slice(format!("cat_{:06}", i % 997).as_bytes());
            present.push(true);
        }
        offsets.push(bytes.len());
    }
    let col = Column::from_utf8_values_with_validity(bytes.clone(), offsets.clone(), validity);

    let mut best_t = u128::MAX;
    for _ in 0..iters {
        let t = std::time::Instant::now();
        let r = col.mode().unwrap();
        best_t = best_t.min(t.elapsed().as_nanos());
        std::hint::black_box(&r);
    }
    let mut best_c = u128::MAX;
    for _ in 0..iters {
        let t = std::time::Instant::now();
        let r = ref_mode_cold(&bytes, &offsets, &present);
        best_c = best_c.min(t.elapsed().as_nanos());
        std::hint::black_box(&r);
    }
    let got = col.mode().unwrap();
    let want = ref_mode_cold(&bytes, &offsets, &present);
    assert_eq!(got.values(), want.values(), "mode mismatch");
    println!(
        "mode utf8_nullable n={n} winners={} NEW={:>7.2}ms CONTROL={:>7.2}ms speedup={:.3}x",
        got.len(),
        best_t as f64 / 1e6,
        best_c as f64 / 1e6,
        best_c as f64 / best_t as f64,
    );
}
