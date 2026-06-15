//! Before/after micro-benchmark for br-frankenpandas-b3n30.
//!
//! Times `Series::expanding().var()` (now the O(n) compensated online
//! `rolling_var_online` over a full-window trailing `Rolling`) against the
//! historical O(n²) per-step two-pass re-fold reproduced inline here, on the
//! same input. Prints p50 of both plus the speedup (Score).
//!
//! Run: cargo run --release -p fp-frame --example bench_expanding_var

use std::time::Instant;

use fp_frame::Series;
use fp_index::IndexLabel;
use fp_types::Scalar;

const N: usize = 100_000;
const RUNS: usize = 7;
const WARMUP: usize = 2;

/// The historical O(n²) expanding variance: per output i, re-fold the entire
/// prefix [0, i] with a two-pass mean + sum-of-squares. This is the code path
/// `Expanding::var` used before b3n30; reproduced so the speedup is measured on
/// identical data in one process.
fn naive_expanding_var(values: &[Scalar], min_periods: usize) -> Vec<Scalar> {
    let mut out = Vec::with_capacity(values.len());
    let mut nums: Vec<f64> = Vec::new();
    for v in values {
        if !v.is_missing()
            && let Ok(x) = v.to_f64()
        {
            nums.push(x);
        }
        if nums.len() < min_periods {
            out.push(Scalar::Null(fp_types::NullKind::NaN));
        } else if nums.len() < 2 {
            out.push(Scalar::Float64(f64::NAN));
        } else {
            let mean = nums.iter().sum::<f64>() / nums.len() as f64;
            let var =
                nums.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (nums.len() - 1) as f64;
            out.push(Scalar::Float64(var));
        }
    }
    out
}

fn p50_ms(mut times: Vec<f64>) -> f64 {
    times.sort_by(|a, b| a.partial_cmp(b).unwrap());
    times[times.len() / 2]
}

fn main() {
    // Deterministic pseudo-random f64 series (xorshift) so both paths see the
    // same non-trivial values (not a monotone ramp that hides cancellation).
    let mut state: u64 = 0x9E3779B97F4A7C15;
    let values: Vec<Scalar> = (0..N)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            Scalar::Float64((state >> 11) as f64 / (1u64 << 53) as f64 * 1000.0 - 500.0)
        })
        .collect();
    let labels: Vec<IndexLabel> = (0..N as i64).map(IndexLabel::from).collect();
    let series = Series::from_values("s", labels, values.clone()).expect("series");

    // Sanity: the two paths agree within tolerance on a small prefix.
    let online_small = series.expanding(Some(1)).var().unwrap();
    let naive_small = naive_expanding_var(&values, 1);
    let mut max_rel = 0.0_f64;
    for (o, n) in online_small.values().iter().zip(&naive_small).take(2000) {
        if let (Scalar::Float64(a), Scalar::Float64(b)) = (o, n)
            && b.is_finite()
            && *b != 0.0
        {
            max_rel = max_rel.max(((a - b) / b).abs());
        }
    }
    println!("isomorphism: max relative diff over first 2000 = {max_rel:.3e}");

    let mut new_times = Vec::new();
    let mut old_times = Vec::new();
    for run in 0..(WARMUP + RUNS) {
        let t = Instant::now();
        let r = series.expanding(Some(1)).var().unwrap();
        std::hint::black_box(&r);
        let new_ms = t.elapsed().as_secs_f64() * 1e3;

        let t = Instant::now();
        let r = naive_expanding_var(&values, 1);
        std::hint::black_box(&r);
        let old_ms = t.elapsed().as_secs_f64() * 1e3;

        if run >= WARMUP {
            new_times.push(new_ms);
            old_times.push(old_ms);
        }
    }

    let new_p50 = p50_ms(new_times);
    let old_p50 = p50_ms(old_times);
    println!("n={N}");
    println!("OLD (O(n^2) two-pass per step): {old_p50:9.3} ms");
    println!("NEW (O(n) online roll_var)    : {new_p50:9.3} ms");
    println!("Score (old/new)               : {:.2}x", old_p50 / new_p50);
}
