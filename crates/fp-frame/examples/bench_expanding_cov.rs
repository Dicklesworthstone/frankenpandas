//! Before/after micro-benchmark for br-frankenpandas-1wc0n.
//!
//! Times `expanding().cov(other)` / `.corr(other)` (now an O(n) online
//! bivariate Welford sweep) against the historical O(n^2) per-step two-pass
//! re-fold reproduced inline, on the same input.
//!
//! Run: cargo run --release -p fp-frame --example bench_expanding_cov

use std::time::Instant;

use fp_frame::Series;
use fp_index::IndexLabel;
use fp_types::{NullKind, Scalar};

const N: usize = 10_000;
const RUNS: usize = 5;
const WARMUP: usize = 1;

fn naive_expanding(a: &[f64], b: &[f64], min_periods: usize, want_corr: bool) -> Vec<Scalar> {
    let mut out = Vec::with_capacity(a.len());
    let mut pa: Vec<f64> = Vec::new();
    let mut pb: Vec<f64> = Vec::new();
    for i in 0..a.len() {
        pa.push(a[i]);
        pb.push(b[i]);
        let n = pa.len() as f64;
        if pa.len() < min_periods.max(2) {
            out.push(Scalar::Null(NullKind::NaN));
            continue;
        }
        let ma = pa.iter().sum::<f64>() / n;
        let mb = pb.iter().sum::<f64>() / n;
        let cov = pa
            .iter()
            .zip(&pb)
            .map(|(x, y)| (x - ma) * (y - mb))
            .sum::<f64>()
            / (n - 1.0);
        if want_corr {
            let sa = (pa.iter().map(|x| (x - ma).powi(2)).sum::<f64>() / (n - 1.0)).sqrt();
            let sb = (pb.iter().map(|y| (y - mb).powi(2)).sum::<f64>() / (n - 1.0)).sqrt();
            if sa == 0.0 || sb == 0.0 {
                out.push(Scalar::Null(NullKind::NaN));
            } else {
                out.push(Scalar::Float64(cov / (sa * sb)));
            }
        } else {
            out.push(Scalar::Float64(cov));
        }
    }
    out
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
    let mut s: u64 = 0x9E3779B97F4A7C15;
    let mut next = || {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        (s >> 11) as f64 / (1u64 << 53) as f64 * 1000.0 - 500.0
    };
    let ra: Vec<f64> = (0..N).map(|_| next()).collect();
    let rb: Vec<f64> = (0..N).map(|_| next()).collect();
    let labels: Vec<IndexLabel> = (0..N as i64).map(IndexLabel::from).collect();
    let sa = Series::from_values(
        "a",
        labels.clone(),
        ra.iter().copied().map(Scalar::Float64).collect(),
    )
    .unwrap();
    let sb = Series::from_values(
        "b",
        labels,
        rb.iter().copied().map(Scalar::Float64).collect(),
    )
    .unwrap();

    let cov_new = time(|| sa.expanding(Some(1)).cov(&sb).unwrap());
    let cov_old = time(|| naive_expanding(&ra, &rb, 1, false));
    let corr_new = time(|| sa.expanding(Some(1)).corr(&sb).unwrap());
    let corr_old = time(|| naive_expanding(&ra, &rb, 1, true));

    println!("n={N}");
    println!(
        "expanding cov  OLD {cov_old:9.3} ms -> NEW {cov_new:9.3} ms = {:.2}x",
        cov_old / cov_new
    );
    println!(
        "expanding corr OLD {corr_old:9.3} ms -> NEW {corr_new:9.3} ms = {:.2}x",
        corr_old / corr_new
    );
}
