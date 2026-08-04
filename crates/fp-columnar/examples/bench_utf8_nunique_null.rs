//! Bench for `Column::nunique` on a NULLABLE contiguous-Utf8 column (`LazyNullableUtf8`)
//! after adding the nullable arm — counts distinct present byte spans directly, instead of
//! the generic `nannunique` that materializes `Vec<Scalar::Utf8>` (a heap `String` per row).
//!
//! NEW = col.nunique(). CONTROL = faithful COLD generic replica (materialize present strings
//! from the backing + FxHashSet<&str>, inside the timed loop) — comparing against a pre-warmed
//! Vec would be dishonest (it skips the very materialization the typed bypass eliminates).
//!
//! Run: cargo run -p fp-columnar --release --example bench_utf8_nunique_null -- 5000000 20

use fp_columnar::{Column, ValidityMask};
use fp_types::Scalar;
use rustc_hash::FxHashSet;

fn ref_nunique_cold(bytes: &[u8], offsets: &[usize], present: &[bool]) -> i64 {
    // (1) materialize present rows into owned Scalar::Utf8, (2) count distinct via &str set.
    let mut vals: Vec<String> = Vec::with_capacity(offsets.len() - 1);
    for (i, w) in offsets.windows(2).enumerate() {
        if present[i] {
            vals.push(std::str::from_utf8(&bytes[w[0]..w[1]]).unwrap().to_string());
        }
    }
    let mut seen: FxHashSet<&str> = FxHashSet::default();
    for s in &vals {
        seen.insert(s.as_str());
    }
    seen.len() as i64
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
        let r = col.nunique();
        best_t = best_t.min(t.elapsed().as_nanos());
        std::hint::black_box(&r);
    }
    let mut best_c = u128::MAX;
    for _ in 0..iters {
        let t = std::time::Instant::now();
        let r = ref_nunique_cold(&bytes, &offsets, &present);
        best_c = best_c.min(t.elapsed().as_nanos());
        std::hint::black_box(&r);
    }
    let got = match col.nunique() {
        Scalar::Int64(v) => v,
        _ => panic!("expected Int64"),
    };
    let want = ref_nunique_cold(&bytes, &offsets, &present);
    assert_eq!(got, want, "nunique mismatch: {got} != {want}");
    println!(
        "nunique utf8_nullable n={n} distinct={got} NEW={:>7.2}ms CONTROL={:>7.2}ms speedup={:.3}x",
        best_t as f64 / 1e6,
        best_c as f64 / 1e6,
        best_c as f64 / best_t as f64,
    );
}
