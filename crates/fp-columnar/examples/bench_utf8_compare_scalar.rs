//! Bench for `Column::compare_scalar` on an all-valid contiguous-Utf8 column vs a Utf8
//! scalar after adding the typed Utf8 arm — compares each row's raw `&[u8]` span against
//! the needle bytes, instead of the generic loop that drives `self.values.iter()` (which
//! materializes a `Vec<Scalar::Utf8>` — one heap `String` per row) to feed scalar_compare.
//!
//! NEW = col.compare_scalar(&needle, op). CONTROL = a replica of the generic loop over the
//! (cached) values() ⇒ conservative lower bound (control does NOT pay the String
//! materialization the real generic path pays on a fresh column — the true win is larger).
//!
//! Run: cargo run -p fp-columnar --release --example bench_utf8_compare_scalar -- 5000000 40

use fp_columnar::{Column, ComparisonOp};
use fp_types::{DType, NullKind, Scalar};

fn ref_eq_col(vals: &[Scalar], needle: &str) -> Column {
    let out: Vec<Scalar> = vals
        .iter()
        .map(|v| match v {
            Scalar::Utf8(a) => Scalar::Bool(a.as_str() == needle),
            _ if v.is_missing() => Scalar::Null(NullKind::Null),
            _ => unreachable!("all-valid Utf8"),
        })
        .collect();
    Column::new(DType::Bool, out).unwrap()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let n: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(5_000_000);
    let iters: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(40);

    // ~1/1000 rows match the needle "cat_000300"; varied so lt/gt are non-trivial too.
    let mut bytes: Vec<u8> = Vec::with_capacity(n * 10);
    let mut offsets: Vec<usize> = Vec::with_capacity(n + 1);
    offsets.push(0);
    for i in 0..n {
        bytes.extend_from_slice(format!("cat_{:06}", i % 1000).as_bytes());
        offsets.push(bytes.len());
    }
    let col = Column::from_utf8_contiguous(bytes, offsets);
    let needle_str = "cat_000300".to_string();
    let needle = Scalar::Utf8(needle_str.clone());

    let vals = col.values().to_vec(); // warm the lazy Scalar-Vec cache for CONTROL

    let mut best_t = u128::MAX;
    for _ in 0..iters {
        let t = std::time::Instant::now();
        let r = col.compare_scalar(&needle, ComparisonOp::Eq).unwrap();
        best_t = best_t.min(t.elapsed().as_nanos());
        std::hint::black_box(&r);
    }
    let mut best_c = u128::MAX;
    for _ in 0..iters {
        let t = std::time::Instant::now();
        let r = ref_eq_col(&vals, &needle_str);
        best_c = best_c.min(t.elapsed().as_nanos());
        std::hint::black_box(&r);
    }
    let got = col.compare_scalar(&needle, ComparisonOp::Eq).unwrap();
    let want = ref_eq_col(&vals, &needle_str);
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
        "compare_scalar utf8_eq n={n} NEW={:>7.2}ms CONTROL={:>7.2}ms speedup={:.3}x",
        best_t as f64 / 1e6,
        best_c as f64 / 1e6,
        best_c as f64 / best_t as f64,
    );
}
