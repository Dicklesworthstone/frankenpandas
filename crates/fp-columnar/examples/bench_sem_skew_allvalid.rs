//! Bench for `Column::sem`/`skew`/`kurt` on an ALL-VALID Float64/Int64 column after
//! reordering the typed present-moments two-pass ahead of the Vec-collecting
//! `typed_collect_finite_f64` path. NEW = col.sem()/skew()/kurt() (two-pass, no
//! alloc). CONTROL = the OLD path: collect finite into a Vec<f64>, then the same
//! formula with separate `.map().sum()` passes. Results asserted equal.
//!
//! Run: cargo run -p fp-columnar --release --example bench_sem_skew_allvalid -- 5000000 20

use fp_columnar::Column;
use fp_types::Scalar;

// OLD path replica: collect finite (all-valid ⇒ drop NaN / v as f64) into a Vec.
fn collect_finite(col: &Column) -> Vec<f64> {
    col.values()
        .iter()
        .filter_map(|v| match v {
            Scalar::Float64(x) if !x.is_nan() => Some(*x),
            Scalar::Int64(x) => Some(*x as f64),
            _ => None,
        })
        .collect()
}

fn sem_vec(col: &Column, ddof: usize) -> Scalar {
    let nums = collect_finite(col);
    let n = nums.len();
    if n <= ddof {
        return Scalar::Null(fp_types::NullKind::NaN);
    }
    let nf = n as f64;
    let mean = nums.iter().sum::<f64>() / nf;
    let sum_sq: f64 = nums.iter().map(|x| (x - mean).powi(2)).sum();
    let std = (sum_sq / (n - ddof) as f64).sqrt();
    Scalar::Float64(std / nf.sqrt())
}

fn skew_vec(col: &Column) -> Scalar {
    let nums = collect_finite(col);
    let n = nums.len() as f64;
    if n < 3.0 {
        return Scalar::Null(fp_types::NullKind::NaN);
    }
    let mean = nums.iter().sum::<f64>() / n;
    let m2: f64 = nums.iter().map(|x| (x - mean).powi(2)).sum();
    let m3: f64 = nums.iter().map(|x| (x - mean).powi(3)).sum();
    let s2 = m2 / (n - 1.0);
    if s2 == 0.0 {
        return Scalar::Float64(0.0);
    }
    Scalar::Float64((n / ((n - 1.0) * (n - 2.0))) * (m3 / s2.powf(1.5)))
}

fn kurt_vec(col: &Column) -> Scalar {
    let nums = collect_finite(col);
    let n = nums.len() as f64;
    if n < 4.0 {
        return Scalar::Null(fp_types::NullKind::NaN);
    }
    let mean = nums.iter().sum::<f64>() / n;
    let m2: f64 = nums.iter().map(|x| (x - mean).powi(2)).sum();
    let m4: f64 = nums.iter().map(|x| (x - mean).powi(4)).sum();
    let s2 = m2 / (n - 1.0);
    if s2 == 0.0 {
        return Scalar::Float64(0.0);
    }
    let adj = (n * (n + 1.0)) / ((n - 1.0) * (n - 2.0) * (n - 3.0));
    let sub = (3.0 * (n - 1.0).powi(2)) / ((n - 2.0) * (n - 3.0));
    Scalar::Float64(adj * (m4 / (s2 * s2)) - sub)
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(5_000_000);
    let iters: usize = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(20);

    let idata: Vec<i64> = (0..n)
        .map(|i| {
            let h = (i as u64).wrapping_mul(2_654_435_761).wrapping_add(999);
            (h % 100_003) as i64 - 50_000
        })
        .collect();
    let fcol = Column::from_f64_values(idata.iter().map(|&x| x as f64 * 0.5).collect());
    let icol = Column::from_i64_values_owned(idata);

    for (label, col) in [("f64_allvalid", &fcol), ("i64_allvalid", &icol)] {
        for op in ["sem", "skew", "kurt"] {
            let new = |c: &Column| match op {
                "sem" => c.sem(1),
                "skew" => c.skew(),
                _ => c.kurt(),
            };
            let old = |c: &Column| match op {
                "sem" => sem_vec(c, 1),
                "skew" => skew_vec(c),
                _ => kurt_vec(c),
            };
            let bench = |f: &dyn Fn(&Column) -> Scalar| -> f64 {
                let mut b = u128::MAX;
                for _ in 0..iters {
                    let t = std::time::Instant::now();
                    let r = f(col);
                    b = b.min(t.elapsed().as_nanos());
                    std::hint::black_box(&r);
                }
                b as f64 / 1e6
            };
            assert_eq!(format!("{:?}", new(col)), format!("{:?}", old(col)), "{label} {op}");
            let t_new = bench(&new);
            let t_old = bench(&old);
            println!(
                "sem_skew {label:>13} {op:>4} n={n} NEW(2pass)={:>6.2}ms OLD(Vec)={:>6.2}ms speedup={:.3}x",
                t_new, t_old, t_old / t_new
            );
        }
    }
}
