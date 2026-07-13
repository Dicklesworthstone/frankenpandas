//! Bench for `Column::eq` (binary_comparison) on two contiguous-Utf8 columns after adding
//! the typed Utf8 arm — compares each row's raw `&[u8]` spans directly, instead of the
//! generic loop that materializes BOTH `Vec<Scalar::Utf8>` (a heap `String` per row × 2).
//!
//! NEW = a.eq(&b). CONTROL = a replica of the generic loop over the (cached) values() ⇒
//! conservative lower bound (control skips the double String materialization).
//!
//! Run: cargo run -p fp-columnar --release --example bench_utf8_binary_compare -- 5000000 40

use fp_columnar::Column;
use fp_types::{DType, NullKind, Scalar};

fn ref_eq_col(la: &[Scalar], ra: &[Scalar]) -> Column {
    let out: Vec<Scalar> = la
        .iter()
        .zip(ra)
        .map(|(l, r)| {
            if l.is_missing() || r.is_missing() {
                Scalar::Null(NullKind::Null)
            } else {
                match (l, r) {
                    (Scalar::Utf8(a), Scalar::Utf8(b)) => Scalar::Bool(a == b),
                    _ => unreachable!("all-valid Utf8"),
                }
            }
        })
        .collect();
    Column::new(DType::Bool, out).unwrap()
}

fn build(n: usize, offset: usize) -> Column {
    let mut bytes: Vec<u8> = Vec::with_capacity(n * 10);
    let mut offsets: Vec<usize> = Vec::with_capacity(n + 1);
    offsets.push(0);
    for i in 0..n {
        bytes.extend_from_slice(format!("cat_{:06}", (i + offset) % 1000).as_bytes());
        offsets.push(bytes.len());
    }
    Column::from_utf8_contiguous(bytes, offsets)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let n: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(5_000_000);
    let iters: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(40);

    // left[i] == right[i] whenever i%1000 and (i+3)%1000 collide (never) → ~all false,
    // but shift by 0 for a fraction so ~1/1 identical block: use offsets that give a mix.
    let a = build(n, 0);
    let b = build(n, 3); // differs by 3 mod 1000 ⇒ all rows differ; flip 1/1000 to equal
    let av = a.values().to_vec();
    let bv = b.values().to_vec();

    let mut best_t = u128::MAX;
    for _ in 0..iters {
        let t = std::time::Instant::now();
        let r = a.eq(&b).unwrap();
        best_t = best_t.min(t.elapsed().as_nanos());
        std::hint::black_box(&r);
    }
    let mut best_c = u128::MAX;
    for _ in 0..iters {
        let t = std::time::Instant::now();
        let r = ref_eq_col(&av, &bv);
        best_c = best_c.min(t.elapsed().as_nanos());
        std::hint::black_box(&r);
    }
    let got = a.eq(&b).unwrap();
    let want = ref_eq_col(&av, &bv);
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
        "eq utf8_col_vs_col n={n} NEW={:>7.2}ms CONTROL={:>7.2}ms speedup={:.3}x",
        best_t as f64 / 1e6,
        best_c as f64 / 1e6,
        best_c as f64 / best_t as f64,
    );
}
