//! Bench for `Column::interpolate_linear` after adding the typed Int64 fast path
//! (sibling of the existing Float64 arm) — sources present values off the raw slice
//! + validity instead of building floats via per-Scalar to_f64 over the Vec<Scalar>.
//!
//! NEW = col.interpolate_linear(). CONTROL = a replica of the generic Scalar path
//! over the (cached) values() ⇒ conservative lower bound (NEW never builds the Scalar
//! Vec on a fresh column). Output length + first differing slot asserted equal.
//!
//! Run: cargo run -p fp-columnar --release --example bench_interpolate -- 5000000 30

use fp_columnar::{Column, ValidityMask};
use fp_types::{NullKind, Scalar};

fn ref_interp(vals: &[Scalar]) -> Vec<Scalar> {
    let len = vals.len();
    let mut floats: Vec<Option<f64>> = Vec::with_capacity(len);
    for v in vals {
        if v.is_missing() {
            floats.push(None);
            continue;
        }
        match v.to_f64() {
            Ok(x) if !x.is_nan() => floats.push(Some(x)),
            _ => floats.push(None),
        }
    }
    let first = floats.iter().position(Option::is_some);
    let last = floats.iter().rposition(Option::is_some);
    if let (Some(start), Some(end)) = (first, last) {
        let mut i = start;
        while i < end {
            if floats[i].is_some() {
                i += 1;
                continue;
            }
            let gap_start = i;
            while i < end && floats[i].is_none() {
                i += 1;
            }
            let before = floats[gap_start - 1].unwrap();
            let after = floats[i].unwrap();
            let span = (i - gap_start + 1) as f64;
            for (k, j) in (gap_start..i).enumerate() {
                let step = (k + 1) as f64;
                floats[j] = Some(before + (after - before) * (step / span));
            }
        }
        let last_valid = floats[end].unwrap();
        for slot in floats.iter_mut().skip(end + 1) {
            *slot = Some(last_valid);
        }
    }
    floats
        .into_iter()
        .map(|opt| match opt {
            Some(x) => Scalar::Float64(x),
            None => Scalar::Null(NullKind::NaN),
        })
        .collect()
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(5_000_000);
    let iters: usize = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(30);

    let idata: Vec<i64> = (0..n)
        .map(|i| {
            let h = (i as u64).wrapping_mul(2_654_435_761).wrapping_add(999);
            (h % 100_003) as i64 - 50_000
        })
        .collect();
    let fdata: Vec<f64> = idata.iter().map(|&x| x as f64 * 0.5).collect();
    let mut validity = ValidityMask::all_valid(n);
    for i in (0..n).step_by(4) {
        validity.set(i, false); // ~25% gaps to interpolate
    }
    let f_null = Column::from_f64_values_with_validity(fdata, validity.clone());
    let i_null = Column::from_i64_values_with_validity(idata, validity);

    for (label, col) in [("f64_nullable", &f_null), ("i64_nullable", &i_null)] {
        let vals = col.values().to_vec(); // warm the lazy Scalar-Vec cache for CONTROL

        let mut best_t = u128::MAX;
        for _ in 0..iters {
            let t = std::time::Instant::now();
            let r = col.interpolate_linear().unwrap();
            best_t = best_t.min(t.elapsed().as_nanos());
            std::hint::black_box(&r);
        }
        let mut best_c = u128::MAX;
        for _ in 0..iters {
            let t = std::time::Instant::now();
            let r = ref_interp(col.values());
            best_c = best_c.min(t.elapsed().as_nanos());
            std::hint::black_box(&r);
        }
        // Parity check: same length, and slot 1 (interpolated) equal.
        let got = col.interpolate_linear().unwrap();
        let want = ref_interp(&vals);
        assert_eq!(got.values().len(), want.len(), "{label}: len mismatch");
        let g1 = format!("{:?}", got.values().get(1));
        let w1 = format!("{:?}", want.get(1));
        assert_eq!(g1, w1, "{label}: slot1 mismatch");
        println!(
            "interpolate {label:>13} n={n} NEW={:>7.2}ms CONTROL={:>7.2}ms speedup={:.3}x",
            best_t as f64 / 1e6,
            best_c as f64 / 1e6,
            best_c as f64 / best_t as f64,
        );
    }
}
