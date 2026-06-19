//! Gauntlet sweep over unmeasured common Series ops to FIND new gaps vs pandas:
//! rank, diff, cumsum, cummax, fillna, clip, nlargest. 2M Float64 (with NaN for fillna).
//! Self-times best-of-N each. Compare vs pandas equivalents.
//!
//! Run: cargo run -p fp-frame --example bench_misc --release -- 2000000 50

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
    let n: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(2_000_000);
    let iters: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(50);
    let labels: Vec<IndexLabel> = (0..n as i64).map(IndexLabel::Int64).collect();
    // Pseudo-shuffled Float64 (LCG) so rank/nlargest do real work; NaN every 10th for fillna.
    let mut st: u64 = 0x2545_f491_4f6c_dd1d;
    let mut nextf = || {
        st = st.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        ((st >> 11) as f64) / (1u64 << 53) as f64
    };
    let vals: Vec<Scalar> = (0..n).map(|_| Scalar::Float64(nextf() * 1e6)).collect();
    let s = Series::from_values("v", labels.clone(), vals).unwrap();
    let mut nan_vals: Vec<Scalar> = (0..n).map(|i| Scalar::Float64(i as f64 * 1.5)).collect();
    for i in (3..n).step_by(10) {
        nan_vals[i] = Scalar::Float64(f64::NAN);
    }
    let s_nan = Series::from_values("v", labels, nan_vals).unwrap();

    let rank = best(iters / 2 + 1, || {
        std::hint::black_box(s.rank("average", true, "keep").expect("rank"));
    });
    let diff = best(iters, || {
        std::hint::black_box(s.diff(1).expect("diff"));
    });
    let cumsum = best(iters, || {
        std::hint::black_box(s.cumsum().expect("cumsum"));
    });
    let cummax = best(iters, || {
        std::hint::black_box(s.cummax().expect("cummax"));
    });
    let fillna = best(iters, || {
        std::hint::black_box(s_nan.fillna(&Scalar::Float64(0.0)).expect("fillna"));
    });
    let clip = best(iters, || {
        std::hint::black_box(s.clip(Some(1e5), Some(9e5)).expect("clip"));
    });
    let nlargest = best(iters, || {
        std::hint::black_box(s.nlargest(20).expect("nlargest"));
    });

    println!(
        "misc n={n}: rank={rank}ns diff={diff}ns cumsum={cumsum}ns cummax={cummax}ns fillna={fillna}ns clip={clip}ns nlargest={nlargest}ns"
    );
}
