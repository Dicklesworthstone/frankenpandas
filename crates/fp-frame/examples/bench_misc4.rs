//! Fourth gauntlet sweep: skew, kurtosis, sem, median, mode. 2M Float64.
//! skew/kurt use numeric_values + multi-pass moment sums — typed single-pass candidates.
//!
//! Run: cargo run -p fp-frame --example bench_misc4 --release -- 2000000 50

use std::time::Instant;

use fp_frame::Series;
use fp_index::IndexLabel;
use fp_types::Scalar;

fn best<F: FnMut()>(iters: usize, mut f: F) -> u128 {
    let mut b = u128::MAX;
    for _ in 0..iters {
        let t = Instant::now();
        f();
        let e = t.elapsed().as_nanos();
        if e < b {
            b = e;
        }
    }
    b
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let n: usize = args
        .get(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(2_000_000);
    let iters: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(50);
    let labels: Vec<IndexLabel> = (0..n as i64).map(IndexLabel::Int64).collect();
    let mut st: u64 = 0xdead_beef_cafe_1234;
    let mut nextf = || {
        st = st.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        ((st >> 11) as f64) / (1u64 << 53) as f64
    };
    let vals: Vec<Scalar> = (0..n).map(|_| Scalar::Float64(nextf() * 1e6)).collect();
    let s = Series::from_values("v", labels.clone(), vals).unwrap();
    // ~1000 distinct for mode
    let mvals: Vec<Scalar> = (0..n)
        .map(|_| Scalar::Float64((nextf() * 1000.0).floor()))
        .collect();
    let sm = Series::from_values("v", labels, mvals).unwrap();

    let skew = best(iters, || {
        std::hint::black_box(s.skew().expect("skew"));
    });
    let kurt = best(iters, || {
        std::hint::black_box(s.kurtosis().expect("kurt"));
    });
    let sem = best(iters, || {
        std::hint::black_box(s.sem().expect("sem"));
    });
    let median = best(iters, || {
        std::hint::black_box(s.median().expect("median"));
    });
    let mode = best(iters / 5 + 1, || {
        std::hint::black_box(sm.mode().expect("mode"));
    });

    println!(
        "misc4 n={n}: skew={skew}ns kurt={kurt}ns sem={sem}ns median={median}ns mode={mode}ns"
    );
}
