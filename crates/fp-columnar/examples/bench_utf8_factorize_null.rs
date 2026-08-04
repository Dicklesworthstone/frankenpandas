//! Bench for `Column::factorize` on a NULLABLE contiguous-Utf8 column (`LazyNullableUtf8`)
//! after adding the nullable default arm — assigns codes over present byte spans directly,
//! instead of the generic `LocalKey` loop that materializes `Vec<Scalar::Utf8>` (a heap
//! String per row).
//!
//! NEW = col.factorize(). CONTROL = faithful COLD generic replica (materialize present strings
//! from the backing + FxHashMap<&str,i64> first-seen code assignment, missing→-1).
//!
//! Run: cargo run -p fp-columnar --release --example bench_utf8_factorize_null -- 5000000 20

use fp_columnar::{Column, ValidityMask};
use fp_types::{DType, Scalar};
use rustc_hash::FxHashMap;

fn ref_factorize_cold(bytes: &[u8], offsets: &[usize], present: &[bool]) -> (Column, Column) {
    let n = offsets.len() - 1;
    // (1) materialize present rows into owned Strings (None marker for missing).
    let mut owned: Vec<Option<String>> = Vec::with_capacity(n);
    for (i, w) in offsets.windows(2).enumerate() {
        if present[i] {
            owned.push(Some(std::str::from_utf8(&bytes[w[0]..w[1]]).unwrap().to_string()));
        } else {
            owned.push(None);
        }
    }
    // (2) first-seen code assignment (missing → -1).
    let mut idx_map: FxHashMap<&str, i64> = FxHashMap::default();
    let mut codes: Vec<Scalar> = Vec::with_capacity(n);
    let mut uniques: Vec<Scalar> = Vec::new();
    for o in &owned {
        match o {
            None => codes.push(Scalar::Int64(-1)),
            Some(s) => match idx_map.get(s.as_str()) {
                Some(&c) => codes.push(Scalar::Int64(c)),
                None => {
                    let c = uniques.len() as i64;
                    idx_map.insert(s.as_str(), c);
                    uniques.push(Scalar::Utf8(s.clone()));
                    codes.push(Scalar::Int64(c));
                }
            },
        }
    }
    (
        Column::new(DType::Int64, codes).unwrap(),
        Column::new(DType::Utf8, uniques).unwrap(),
    )
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
        let r = col.factorize().unwrap();
        best_t = best_t.min(t.elapsed().as_nanos());
        std::hint::black_box(&r);
    }
    let mut best_c = u128::MAX;
    for _ in 0..iters {
        let t = std::time::Instant::now();
        let r = ref_factorize_cold(&bytes, &offsets, &present);
        best_c = best_c.min(t.elapsed().as_nanos());
        std::hint::black_box(&r);
    }
    let (gc, gu) = col.factorize().unwrap();
    let (wc, wu) = ref_factorize_cold(&bytes, &offsets, &present);
    assert_eq!(gc.values(), wc.values(), "codes mismatch");
    assert_eq!(gu.values(), wu.values(), "uniques mismatch");
    println!(
        "factorize utf8_nullable n={n} uniques={} NEW={:>7.2}ms CONTROL={:>7.2}ms speedup={:.3}x",
        gu.len(),
        best_t as f64 / 1e6,
        best_c as f64 / 1e6,
        best_c as f64 / best_t as f64,
    );
}
