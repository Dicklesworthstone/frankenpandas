//! Before/after micro-benchmark for br-frankenpandas-g6qa2.
//!
//! Times `rolling(w).sem()` and `expanding().sem()` (now the O(n) online
//! variance sweep with a `sem = std/√nobs` post-transform) against the
//! historical per-window two-pass re-fold reproduced inline, on the same input.
//!
//! Run: cargo run --release -p fp-frame --example bench_window_sem

use std::time::Instant;

use fp_frame::Series;
use fp_index::IndexLabel;
use fp_types::{NullKind, Scalar};

const N: usize = 100_000;
const WINDOW: usize = 50;
const RUNS: usize = 7;
const WARMUP: usize = 2;

fn naive_sem_window(nums: &[f64], min_periods: usize) -> Scalar {
    if nums.len() < min_periods {
        return Scalar::Null(NullKind::NaN);
    }
    if nums.len() < 2 {
        return Scalar::Float64(f64::NAN);
    }
    let mean = nums.iter().sum::<f64>() / nums.len() as f64;
    let var = nums.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (nums.len() - 1) as f64;
    Scalar::Float64(var.sqrt() / (nums.len() as f64).sqrt())
}

fn naive_rolling_sem(vals: &[f64], window: usize, min_periods: usize) -> Vec<Scalar> {
    (0..vals.len())
        .map(|i| {
            let start = (i + 1).saturating_sub(window);
            naive_sem_window(&vals[start..=i], min_periods)
        })
        .collect()
}

fn naive_expanding_sem(vals: &[f64], min_periods: usize) -> Vec<Scalar> {
    (0..vals.len())
        .map(|i| naive_sem_window(&vals[0..=i], min_periods))
        .collect()
}

fn p50_ms(mut t: Vec<f64>) -> f64 {
    t.sort_by(|a, b| a.partial_cmp(b).unwrap());
    t[t.len() / 2]
}

fn time<T>(f: impl Fn() -> T) -> f64 {
    let mut times = Vec::new();
    for run in 0..(WARMUP + RUNS) {
        let t = Instant::now();
        let r = f();
        std::hint::black_box(&r);
        if run >= WARMUP {
            times.push(t.elapsed().as_secs_f64() * 1e3);
        }
    }
    p50_ms(times)
}

fn main() {
    let mut state: u64 = 0x9E3779B97F4A7C15;
    let raw: Vec<f64> = (0..N)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 11) as f64 / (1u64 << 53) as f64 * 1000.0 - 500.0
        })
        .collect();
    let values: Vec<Scalar> = raw.iter().copied().map(Scalar::Float64).collect();
    let labels: Vec<IndexLabel> = (0..N as i64).map(IndexLabel::from).collect();
    let series = Series::from_values("s", labels, values).expect("series");

    let roll_new = time(|| series.rolling(WINDOW, Some(1)).sem().unwrap());
    let roll_old = time(|| naive_rolling_sem(&raw, WINDOW, 1));
    println!("n={N} window={WINDOW}");
    println!(
        "rolling sem  OLD {roll_old:9.3} ms -> NEW {roll_new:9.3} ms = {:.2}x",
        roll_old / roll_new
    );

    let exp_new = time(|| series.expanding(Some(1)).sem().unwrap());
    let exp_old = time(|| naive_expanding_sem(&raw, 1));
    println!(
        "expanding sem OLD {exp_old:9.3} ms -> NEW {exp_new:9.3} ms = {:.2}x",
        exp_old / exp_new
    );
}
