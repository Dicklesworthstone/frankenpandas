//! Bench for `Column::cov`/`corr` after adding the typed pairwise fast path (fold
//! both raw slices via get_finite, skipping the Vec<Scalar> materialization of
//! BOTH columns + per-Scalar to_f64).
//!
//! NEW = a.cov(b)/a.corr(b) (typed). CONTROL = the old Scalar loop replicated over
//! a.values()/b.values() (cached ⇒ conservative — NEW never materializes either
//! Scalar Vec on fresh columns). Results asserted equal.
//!
//! Run: cargo run -p fp-columnar --release --example bench_cov_corr_typed -- 5000000 15

use fp_columnar::{Column, ValidityMask};
use fp_types::Scalar;

fn present(v: &Scalar) -> Option<f64> {
    match v.to_f64() {
        Ok(x) if x.is_finite() => Some(x),
        _ => None,
    }
}

fn cov_scalar(a: &Column, b: &Column, ddof: usize) -> Scalar {
    let (av, bv) = (a.values(), b.values());
    let n = av.len().min(bv.len());
    let (mut sx, mut sy, mut c) = (0.0f64, 0.0f64, 0usize);
    for i in 0..n {
        if let (Some(x), Some(y)) = (present(&av[i]), present(&bv[i])) {
            sx += x;
            sy += y;
            c += 1;
        }
    }
    if c <= ddof {
        return Scalar::Null(fp_types::NullKind::NaN);
    }
    let (mx, my) = (sx / c as f64, sy / c as f64);
    let mut cs = 0.0f64;
    for i in 0..n {
        if let (Some(x), Some(y)) = (present(&av[i]), present(&bv[i])) {
            cs += (x - mx) * (y - my);
        }
    }
    Scalar::Float64(cs / (c - ddof) as f64)
}

fn corr_scalar(a: &Column, b: &Column) -> Scalar {
    let (av, bv) = (a.values(), b.values());
    let n = av.len().min(bv.len());
    let (mut sx, mut sy, mut sxx, mut syy, mut sxy, mut c) =
        (0.0f64, 0.0f64, 0.0f64, 0.0f64, 0.0f64, 0usize);
    for i in 0..n {
        if let (Some(x), Some(y)) = (present(&av[i]), present(&bv[i])) {
            sx += x;
            sy += y;
            sxx += x * x;
            syy += y * y;
            sxy += x * y;
            c += 1;
        }
    }
    if c < 2 {
        return Scalar::Null(fp_types::NullKind::NaN);
    }
    let nf = c as f64;
    let num = nf * sxy - sx * sy;
    let dx = (nf * sxx - sx * sx).sqrt();
    let dy = (nf * syy - sy * sy).sqrt();
    if dx == 0.0 || dy == 0.0 {
        return Scalar::Null(fp_types::NullKind::NaN);
    }
    Scalar::Float64(num / (dx * dy))
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(5_000_000);
    let iters: usize = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(15);

    let xi: Vec<i64> = (0..n)
        .map(|i| ((i as u64).wrapping_mul(2_654_435_761).wrapping_add(7) % 100_003) as i64 - 50_000)
        .collect();
    let yi: Vec<i64> = (0..n)
        .map(|i| ((i as u64).wrapping_mul(0x9E37_79B9).wrapping_add(3) % 100_003) as i64 - 50_000)
        .collect();
    let xf: Vec<f64> = xi.iter().map(|&v| v as f64 * 0.5).collect();
    let yf: Vec<f64> = yi.iter().map(|&v| v as f64 * 0.25).collect();
    let mut val = ValidityMask::all_valid(n);
    for i in (0..n).step_by(7) {
        val.set(i, false);
    }

    let f_a = Column::from_f64_values(xf);
    let f_b = Column::from_f64_values(yf);
    let i_a = Column::from_i64_values_with_validity(xi.clone(), val.clone());
    let i_b = Column::from_i64_values_with_validity(yi, val);

    for (label, a, b) in [
        ("f64_allvalid", &f_a, &f_b),
        ("i64_nullable", &i_a, &i_b),
    ] {
        for op in ["cov", "corr"] {
            let is_cov = op == "cov";
            let mut best_t = u128::MAX;
            for _ in 0..iters {
                let t = std::time::Instant::now();
                let r = if is_cov { a.cov(b) } else { a.corr(b) };
                best_t = best_t.min(t.elapsed().as_nanos());
                std::hint::black_box(&r);
            }
            let mut best_c = u128::MAX;
            for _ in 0..iters {
                let t = std::time::Instant::now();
                let r = if is_cov { cov_scalar(a, b, 1) } else { corr_scalar(a, b) };
                best_c = best_c.min(t.elapsed().as_nanos());
                std::hint::black_box(&r);
            }
            let (nr, cr) = if is_cov {
                (a.cov(b), cov_scalar(a, b, 1))
            } else {
                (a.corr(b), corr_scalar(a, b))
            };
            assert_eq!(format!("{nr:?}"), format!("{cr:?}"), "{label} {op}");
            println!(
                "cov_corr {label:>13} {op:>4} n={n} NEW={:>7.2}ms CONTROL={:>7.2}ms speedup={:.3}x",
                best_t as f64 / 1e6,
                best_c as f64 / 1e6,
                best_c as f64 / best_t as f64,
            );
        }
    }
}
