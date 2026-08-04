//! Bench for `Column::sort_values` on a NULLABLE contiguous-Utf8 column (`LazyNullableUtf8`)
//! after adding the nullable arm — radix-sorts present byte spans + gathers into a contiguous
//! buffer (na-last), instead of the generic comparator that materializes `Vec<Scalar::Utf8>`
//! (a heap String per row) then clones a String per output row.
//!
//! NEW = col.sort_values(true) [typed arm]. COLD = materialize + na-last comparator + clone
//! (the old cold cost). WARM = same but over a pre-materialized Vec (no input materialization),
//! to check whether the typed path regresses an already-warm column.
//!
//! Run: cargo run -p fp-columnar --release --example bench_utf8_sort_null -- 5000000 20

use fp_columnar::{Column, ValidityMask};
use fp_types::{DType, NullKind, Scalar};

fn na_last_cmp(a: &Option<String>, b: &Option<String>, ascending: bool) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    match (a, b) {
        (Some(x), Some(y)) => {
            if ascending {
                x.cmp(y)
            } else {
                y.cmp(x)
            }
        }
        (Some(_), None) => Ordering::Less,    // present before missing
        (None, Some(_)) => Ordering::Greater, // missing last
        (None, None) => Ordering::Equal,
    }
}

fn build_sorted_column(mut items: Vec<Option<String>>, ascending: bool) -> Column {
    items.sort_by(|a, b| na_last_cmp(a, b, ascending)); // stable
    let out: Vec<Scalar> = items
        .into_iter()
        .map(|o| match o {
            Some(s) => Scalar::Utf8(s),
            None => Scalar::Null(NullKind::Null),
        })
        .collect();
    Column::new(DType::Utf8, out).unwrap()
}

fn materialize(bytes: &[u8], offsets: &[usize], present: &[bool]) -> Vec<Option<String>> {
    offsets
        .windows(2)
        .enumerate()
        .map(|(i, w)| {
            if present[i] {
                Some(std::str::from_utf8(&bytes[w[0]..w[1]]).unwrap().to_string())
            } else {
                None
            }
        })
        .collect()
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
    let warm_items = materialize(&bytes, &offsets, &present); // pre-materialized for WARM

    let mut best_t = u128::MAX;
    for _ in 0..iters {
        let t = std::time::Instant::now();
        let r = col.sort_values(true).unwrap();
        best_t = best_t.min(t.elapsed().as_nanos());
        std::hint::black_box(&r);
    }
    let mut best_cold = u128::MAX;
    for _ in 0..iters {
        let t = std::time::Instant::now();
        let items = materialize(&bytes, &offsets, &present);
        let r = build_sorted_column(items, true);
        best_cold = best_cold.min(t.elapsed().as_nanos());
        std::hint::black_box(&r);
    }
    let mut best_warm = u128::MAX;
    for _ in 0..iters {
        let t = std::time::Instant::now();
        let r = build_sorted_column(warm_items.clone(), true);
        best_warm = best_warm.min(t.elapsed().as_nanos());
        std::hint::black_box(&r);
    }
    let got = col.sort_values(true).unwrap();
    let want = build_sorted_column(materialize(&bytes, &offsets, &present), true);
    assert_eq!(got.values(), want.values(), "sort mismatch");
    println!(
        "sort_values utf8_nullable n={n} NEW={:>7.2}ms COLD={:>7.2}ms(={:.2}x) WARM={:>7.2}ms(={:.2}x)",
        best_t as f64 / 1e6,
        best_cold as f64 / 1e6,
        best_cold as f64 / best_t as f64,
        best_warm as f64 / 1e6,
        best_warm as f64 / best_t as f64,
    );
}
