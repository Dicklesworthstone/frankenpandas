//! Bench for `Column::where_cond` on a NULLABLE contiguous-Utf8 column (`LazyNullableUtf8`)
//! vs a Utf8 scalar `other`, after adding the typed Utf8 arm — builds the select output as one
//! contiguous buffer (self span / other bytes), instead of the generic loop that clones a heap
//! String per row.
//!
//! NEW = col.where_cond(&cond, &other) [typed arm]. COLD = materialize + per-row Scalar clone.
//! WARM = same over a pre-materialized Vec.
//!
//! Run: cargo run -p fp-columnar --release --example bench_utf8_where_null -- 5000000 20

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

fn where_clone(vals: &[Scalar], cond: &[bool], other: &Scalar) -> Column {
    let out: Vec<Scalar> = vals
        .iter()
        .zip(cond)
        .map(|(v, &c)| if c { v.clone() } else { other.clone() })
        .collect();
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
    let mut cbits: Vec<bool> = Vec::with_capacity(n);
    for i in 0..n {
        if i % 5 == 0 {
            validity.set(i, false);
            present.push(false);
        } else {
            bytes.extend_from_slice(format!("cat_{:06}", i % 1000).as_bytes());
            present.push(true);
        }
        cbits.push(i % 2 == 0); // ~50% keep-self, ~50% take-other
        offsets.push(bytes.len());
    }
    let col = Column::from_utf8_values_with_validity(bytes.clone(), offsets.clone(), validity);
    let cond = Column::from_bool_values(cbits.clone());
    let other = Scalar::Utf8("DEFAULT".to_string());
    let warm_vals = materialize(&bytes, &offsets, &present);

    let mut best_t = u128::MAX;
    for _ in 0..iters {
        let t = std::time::Instant::now();
        let r = col.where_cond(&cond, &other).unwrap();
        best_t = best_t.min(t.elapsed().as_nanos());
        std::hint::black_box(&r);
    }
    let mut best_cold = u128::MAX;
    for _ in 0..iters {
        let t = std::time::Instant::now();
        let vals = materialize(&bytes, &offsets, &present);
        let r = where_clone(&vals, &cbits, &other);
        best_cold = best_cold.min(t.elapsed().as_nanos());
        std::hint::black_box(&r);
    }
    let mut best_warm = u128::MAX;
    for _ in 0..iters {
        let t = std::time::Instant::now();
        let r = where_clone(&warm_vals, &cbits, &other);
        best_warm = best_warm.min(t.elapsed().as_nanos());
        std::hint::black_box(&r);
    }
    let got = col.where_cond(&cond, &other).unwrap();
    let want = where_clone(&warm_vals, &cbits, &other);
    let gv = got.values();
    let wv = want.values();
    assert_eq!(gv.len(), wv.len());
    for k in 0..n {
        assert_eq!(format!("{:?}", gv.get(k)), format!("{:?}", wv.get(k)), "slot {k}");
    }
    println!(
        "where_cond utf8_nullable n={n} NEW={:>7.2}ms COLD={:>7.2}ms(={:.2}x) WARM={:>7.2}ms(={:.2}x)",
        best_t as f64 / 1e6,
        best_cold as f64 / 1e6,
        best_cold as f64 / best_t as f64,
        best_warm as f64 / 1e6,
        best_warm as f64 / best_t as f64,
    );
}
