//! Bench for `Column::clip` on a nullable Int64 column after adding the typed arm
//! (sibling of the nullable Float64 arm) — clamps over the raw &[i64] in parallel +
//! reuses the input validity mask, instead of the generic per-Scalar to_f64 loop that
//! boxes a Vec<Scalar> output + Self::new (normalize + validity scan + coercion check).
//!
//! NEW = col.clip(lo, hi). CONTROL = a replica of the old generic loop (Vec<Scalar> +
//! Column::new) over the (cached) values() ⇒ conservative lower bound. Output slot 0/1
//! asserted equal.
//!
//! Run: cargo run -p fp-columnar --release --example bench_clip_i64null -- 5000000 30

use fp_columnar::{Column, ValidityMask};
use fp_types::{DType, Scalar};

fn ref_clip_col(vals: &[Scalar], lo: Option<f64>, hi: Option<f64>) -> Column {
    let mut out = Vec::with_capacity(vals.len());
    for v in vals {
        if v.is_missing() {
            out.push(v.clone());
            continue;
        }
        let mut x = v.to_f64().unwrap();
        if let Some(l) = lo
            && x < l
        {
            x = l;
        }
        if let Some(h) = hi
            && x > h
        {
            x = h;
        }
        out.push(Scalar::Float64(x));
    }
    Column::new(DType::Float64, out).unwrap()
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(5_000_000);
    let iters: usize = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(30);
    let (lo, hi) = (Some(-25_000.0f64), Some(25_000.0f64));

    let idata: Vec<i64> = (0..n)
        .map(|i| {
            let h = (i as u64).wrapping_mul(2_654_435_761).wrapping_add(999);
            (h % 100_003) as i64 - 50_000
        })
        .collect();
    let mut validity = ValidityMask::all_valid(n);
    for i in (0..n).step_by(5) {
        validity.set(i, false);
    }
    let i_null = Column::from_i64_values_with_validity(idata, validity);

    let vals = i_null.values().to_vec(); // warm the lazy Scalar-Vec cache for CONTROL

    let mut best_t = u128::MAX;
    for _ in 0..iters {
        let t = std::time::Instant::now();
        let r = i_null.clip(lo, hi).unwrap();
        best_t = best_t.min(t.elapsed().as_nanos());
        std::hint::black_box(&r);
    }
    let mut best_c = u128::MAX;
    for _ in 0..iters {
        let t = std::time::Instant::now();
        let r = ref_clip_col(i_null.values(), lo, hi);
        best_c = best_c.min(t.elapsed().as_nanos());
        std::hint::black_box(&r);
    }
    let got = i_null.clip(lo, hi).unwrap();
    let want = ref_clip_col(&vals, lo, hi);
    for k in [0usize, 1] {
        assert_eq!(
            format!("{:?}", got.values().get(k)),
            format!("{:?}", want.values().get(k)),
            "slot {k} mismatch"
        );
    }
    println!(
        "clip i64_nullable n={n} NEW={:>7.2}ms CONTROL={:>7.2}ms speedup={:.3}x",
        best_t as f64 / 1e6,
        best_c as f64 / 1e6,
        best_c as f64 / best_t as f64,
    );
}
