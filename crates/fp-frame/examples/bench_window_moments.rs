//! Before/after micro-benchmark for br-frankenpandas-g2veb.
//!
//! Times rolling/expanding skew and kurt against the historical per-window or
//! per-prefix two-pass moment formulas reproduced inline.
//!
//! Run: cargo run --release -p fp-frame --example bench_window_moments

use std::time::Instant;

use fp_frame::Series;
use fp_index::IndexLabel;
use fp_types::{NullKind, Scalar};

const ROLLING_N: usize = 100_000;
const EXPANDING_N: usize = 8_000;
const WINDOW: usize = 50;
const RUNS: usize = 5;
const WARMUP: usize = 1;

fn skew_window(nums: &[f64], min_periods: usize) -> Scalar {
    if nums.len() < min_periods {
        return Scalar::Null(NullKind::NaN);
    }
    let n = nums.len() as f64;
    if n < 3.0 {
        return Scalar::Float64(f64::NAN);
    }
    let mean = nums.iter().sum::<f64>() / n;
    let m2: f64 = nums.iter().map(|v| (v - mean).powi(2)).sum();
    let m3: f64 = nums.iter().map(|v| (v - mean).powi(3)).sum();
    if nums.iter().all(|&v| v == nums[0]) {
        return Scalar::Float64(0.0);
    }
    if m2 / n <= 1e-14 {
        return Scalar::Float64(f64::NAN);
    }
    let s2 = m2 / (n - 1.0);
    let s3 = s2.powf(1.5);
    Scalar::Float64((n / ((n - 1.0) * (n - 2.0))) * (m3 / s3))
}

fn kurt_window(nums: &[f64], min_periods: usize) -> Scalar {
    if nums.len() < min_periods {
        return Scalar::Null(NullKind::NaN);
    }
    let n = nums.len() as f64;
    if n < 4.0 {
        return Scalar::Float64(f64::NAN);
    }
    let mean = nums.iter().sum::<f64>() / n;
    let m2: f64 = nums.iter().map(|v| (v - mean).powi(2)).sum();
    let m4: f64 = nums.iter().map(|v| (v - mean).powi(4)).sum();
    if nums.iter().all(|&v| v == nums[0]) {
        return Scalar::Float64(-3.0);
    }
    if m2 / n <= 1e-14 {
        return Scalar::Float64(f64::NAN);
    }
    let s2 = m2 / (n - 1.0);
    let adj = (n * (n + 1.0)) / ((n - 1.0) * (n - 2.0) * (n - 3.0));
    let sub = (3.0 * (n - 1.0).powi(2)) / ((n - 2.0) * (n - 3.0));
    Scalar::Float64(adj * (m4 / (s2 * s2)) - sub)
}

fn naive_rolling(vals: &[f64], min_periods: usize, want_kurt: bool) -> Vec<Scalar> {
    (0..vals.len())
        .map(|i| {
            let start = (i + 1).saturating_sub(WINDOW);
            let nums = &vals[start..=i];
            if want_kurt {
                kurt_window(nums, min_periods)
            } else {
                skew_window(nums, min_periods)
            }
        })
        .collect()
}

fn naive_expanding(vals: &[f64], min_periods: usize, want_kurt: bool) -> Vec<Scalar> {
    (0..vals.len())
        .map(|i| {
            let nums = &vals[0..=i];
            if want_kurt {
                kurt_window(nums, min_periods)
            } else {
                skew_window(nums, min_periods)
            }
        })
        .collect()
}

fn p50_ms(mut times: Vec<f64>) -> f64 {
    times.sort_by(|a, b| a.partial_cmp(b).unwrap());
    times[times.len() / 2]
}

fn time<T>(f: impl Fn() -> T) -> f64 {
    let mut times = Vec::new();
    for run in 0..(WARMUP + RUNS) {
        let start = Instant::now();
        let r = f();
        std::hint::black_box(&r);
        if run >= WARMUP {
            times.push(start.elapsed().as_secs_f64() * 1e3);
        }
    }
    p50_ms(times)
}

fn data(n: usize) -> Vec<f64> {
    let mut state: u64 = 0x9E3779B97F4A7C15;
    (0..n)
        .map(|i| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let jitter = (state >> 11) as f64 / (1u64 << 53) as f64;
            ((i as f64) * 0.017).sin() * 100.0 + (i % 997) as f64 * 0.01 + jitter
        })
        .collect()
}

fn series(raw: &[f64]) -> Series {
    let values: Vec<Scalar> = raw.iter().copied().map(Scalar::Float64).collect();
    let labels: Vec<IndexLabel> = (0..raw.len() as i64).map(IndexLabel::from).collect();
    Series::from_values("s", labels, values).expect("series")
}

fn main() {
    let rolling_raw = data(ROLLING_N);
    let rolling_series = series(&rolling_raw);
    let roll_skew_new = time(|| rolling_series.rolling(WINDOW, Some(1)).skew().unwrap());
    let roll_skew_old = time(|| naive_rolling(&rolling_raw, 1, false));
    let roll_kurt_new = time(|| rolling_series.rolling(WINDOW, Some(1)).kurt().unwrap());
    let roll_kurt_old = time(|| naive_rolling(&rolling_raw, 1, true));

    let expanding_raw = data(EXPANDING_N);
    let expanding_series = series(&expanding_raw);
    let exp_skew_new = time(|| expanding_series.expanding(Some(1)).skew().unwrap());
    let exp_skew_old = time(|| naive_expanding(&expanding_raw, 1, false));
    let exp_kurt_new = time(|| expanding_series.expanding(Some(1)).kurt().unwrap());
    let exp_kurt_old = time(|| naive_expanding(&expanding_raw, 1, true));

    println!("rolling_n={ROLLING_N} expanding_n={EXPANDING_N} window={WINDOW}");
    println!(
        "rolling skew  OLD {roll_skew_old:9.3} ms -> NEW {roll_skew_new:9.3} ms = {:.2}x",
        roll_skew_old / roll_skew_new
    );
    println!(
        "rolling kurt  OLD {roll_kurt_old:9.3} ms -> NEW {roll_kurt_new:9.3} ms = {:.2}x",
        roll_kurt_old / roll_kurt_new
    );
    println!(
        "expanding skew OLD {exp_skew_old:9.3} ms -> NEW {exp_skew_new:9.3} ms = {:.2}x",
        exp_skew_old / exp_skew_new
    );
    println!(
        "expanding kurt OLD {exp_kurt_old:9.3} ms -> NEW {exp_kurt_new:9.3} ms = {:.2}x",
        exp_kurt_old / exp_kurt_new
    );
}
