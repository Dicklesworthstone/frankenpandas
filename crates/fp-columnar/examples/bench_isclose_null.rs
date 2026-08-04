//! Bench for `Column::isclose` on nullable columns after adding the typed pairwise
//! Bool arm — folds the closeness test off both raw slices via get_present into a
//! Bool buffer, instead of the generic per-Scalar loop that boxes a Vec<Scalar> +
//! Column::new.
//!
//! NEW = a.isclose(&b, rtol, atol). CONTROL = a replica of the old generic loop
//! (Vec<Scalar> + Column::new) over the (cached) values() of both ⇒ conservative
//! lower bound.
//!
//! Run: cargo run -p fp-columnar --release --example bench_isclose_null -- 5000000 30

use fp_columnar::{Column, ValidityMask};
use fp_types::{DType, Scalar};

fn ref_isclose_col(a: &[Scalar], b: &[Scalar], rtol: f64, atol: f64) -> Column {
    let out: Vec<Scalar> = a
        .iter()
        .zip(b.iter())
        .map(|(x, y)| {
            if x.is_missing() || y.is_missing() {
                Scalar::Bool(false)
            } else {
                let af = x.to_f64().unwrap();
                let bf = y.to_f64().unwrap();
                Scalar::Bool((af - bf).abs() <= atol + rtol * bf.abs())
            }
        })
        .collect();
    Column::new(DType::Bool, out).unwrap()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let n: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(5_000_000);
    let iters: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(30);
    let (rtol, atol) = (1e-5f64, 1e-8f64);

    let adata: Vec<f64> = (0..n)
        .map(|i| {
            let h = (i as u64).wrapping_mul(2_654_435_761).wrapping_add(999);
            (h % 100_003) as f64 * 0.5 - 25_000.0
        })
        .collect();
    // b close to a for ~half (so both branches of the comparison run).
    let bdata: Vec<i64> = adata.iter().map(|&x| x as i64).collect();
    let mut va = ValidityMask::all_valid(n);
    let mut vb = ValidityMask::all_valid(n);
    for i in (0..n).step_by(5) {
        va.set(i, false);
    }
    for i in (0..n).step_by(7) {
        vb.set(i, false);
    }
    let a = Column::from_f64_values_with_validity(adata, va);
    let b = Column::from_i64_values_with_validity(bdata, vb);

    let av = a.values().to_vec(); // warm both lazy Scalar-Vec caches for CONTROL
    let bv = b.values().to_vec();

    let mut best_t = u128::MAX;
    for _ in 0..iters {
        let t = std::time::Instant::now();
        let r = a.isclose(&b, rtol, atol).unwrap();
        best_t = best_t.min(t.elapsed().as_nanos());
        std::hint::black_box(&r);
    }
    let mut best_c = u128::MAX;
    for _ in 0..iters {
        let t = std::time::Instant::now();
        let r = ref_isclose_col(a.values(), b.values(), rtol, atol);
        best_c = best_c.min(t.elapsed().as_nanos());
        std::hint::black_box(&r);
    }
    let got = a.isclose(&b, rtol, atol).unwrap();
    let want = ref_isclose_col(&av, &bv, rtol, atol);
    for k in [0usize, 1, 2] {
        assert_eq!(
            format!("{:?}", got.values().get(k)),
            format!("{:?}", want.values().get(k)),
            "slot {k} mismatch"
        );
    }
    println!(
        "isclose f64null_x_i64null n={n} NEW={:>7.2}ms CONTROL={:>7.2}ms speedup={:.3}x",
        best_t as f64 / 1e6,
        best_c as f64 / 1e6,
        best_c as f64 / best_t as f64,
    );
}
