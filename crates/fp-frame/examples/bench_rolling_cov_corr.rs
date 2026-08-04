//! Before/after micro-benchmark for br-frankenpandas-190gg.
//!
//! Times `rolling().cov(other)` / `.corr(other)` against the historical
//! per-window two-pass refold reproduced inline on the same all-valid input.
//!
//! Run: cargo run --release -p fp-frame --example bench_rolling_cov_corr

use std::time::Instant;

use fp_frame::Series;
use fp_index::IndexLabel;
use fp_types::{NullKind, Scalar};

const N: usize = 100_000;
const WINDOW: usize = 100;
const RUNS: usize = 5;
const WARMUP: usize = 1;

fn naive_rolling(a: &[f64], b: &[f64], min_periods: usize, want_corr: bool) -> Vec<Scalar> {
    (0..a.len())
        .map(|i| {
            let start = (i + 1).saturating_sub(WINDOW);
            let nobs = i + 1 - start;
            if nobs < min_periods || nobs < 2 {
                return Scalar::Null(NullKind::NaN);
            }

            let n = nobs as f64;
            let (mut sum_a, mut sum_b) = (0.0, 0.0);
            for j in start..=i {
                sum_a += a[j];
                sum_b += b[j];
            }
            let mean_a = sum_a / n;
            let mean_b = sum_b / n;
            let (mut cov_num, mut var_a, mut var_b) = (0.0, 0.0, 0.0);
            for j in start..=i {
                let da = a[j] - mean_a;
                let db = b[j] - mean_b;
                cov_num += da * db;
                var_a += da * da;
                var_b += db * db;
            }
            let cov = cov_num / (n - 1.0);
            if want_corr {
                let std_a = (var_a / (n - 1.0)).sqrt();
                let std_b = (var_b / (n - 1.0)).sqrt();
                if std_a == 0.0 || std_b == 0.0 {
                    Scalar::Null(NullKind::NaN)
                } else {
                    Scalar::Float64(cov / (std_a * std_b))
                }
            } else {
                Scalar::Float64(cov)
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

fn data(n: usize, seed: u64) -> Vec<f64> {
    let mut state = seed;
    (0..n)
        .map(|i| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let jitter = (state >> 11) as f64 / (1u64 << 53) as f64;
            ((i as f64) * 0.013).cos() * 80.0 + (i % 769) as f64 * 0.05 + jitter
        })
        .collect()
}

fn series(name: &str, raw: &[f64]) -> Series {
    let values: Vec<Scalar> = raw.iter().copied().map(Scalar::Float64).collect();
    let labels: Vec<IndexLabel> = (0..raw.len() as i64).map(IndexLabel::from).collect();
    Series::from_values(name, labels, values).expect("series")
}

fn main() {
    let a_raw = data(N, 0x9E3779B97F4A7C15);
    let b_raw = data(N, 0x6A09E667F3BCC909);
    let a = series("a", &a_raw);
    let b = series("b", &b_raw);

    let cov_new = time(|| a.rolling(WINDOW, Some(1)).cov(&b).unwrap());
    let cov_old = time(|| naive_rolling(&a_raw, &b_raw, 1, false));
    let corr_new = time(|| a.rolling(WINDOW, Some(1)).corr(&b).unwrap());
    let corr_old = time(|| naive_rolling(&a_raw, &b_raw, 1, true));

    println!("n={N} window={WINDOW}");
    println!(
        "rolling cov  OLD {cov_old:9.3} ms -> NEW {cov_new:9.3} ms = {:.2}x",
        cov_old / cov_new
    );
    println!(
        "rolling corr OLD {corr_old:9.3} ms -> NEW {corr_new:9.3} ms = {:.2}x",
        corr_old / corr_new
    );
}
