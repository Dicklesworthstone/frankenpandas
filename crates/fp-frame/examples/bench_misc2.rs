//! Second gauntlet sweep for new gaps: idxmax, idxmin, nunique, abs, round, pct_change,
//! cumprod, cummin. 2M Float64. Self-times best-of-N. idxmax/idxmin are Scalar-based
//! (values() + semantic_cmp) — typed-path candidates like nlargest.
//!
//! Run: cargo run -p fp-frame --example bench_misc2 --release -- 2000000 50

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
    let mut st: u64 = 0x9e37_79b9_7f4a_7c15;
    let mut nextf = || {
        st = st.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        ((st >> 11) as f64) / (1u64 << 53) as f64
    };
    let vals: Vec<Scalar> = (0..n)
        .map(|_| Scalar::Float64(nextf() * 1e6 + 1.0))
        .collect();
    let s = Series::from_values("v", labels, vals).unwrap();

    let idxmax = best(iters, || {
        std::hint::black_box(s.idxmax().expect("idxmax"));
    });
    let idxmin = best(iters, || {
        std::hint::black_box(s.idxmin().expect("idxmin"));
    });
    let nunique = best(iters / 2 + 1, || {
        std::hint::black_box(s.nunique());
    });
    let abs = best(iters, || {
        std::hint::black_box(s.abs().expect("abs"));
    });
    let round = best(iters, || {
        std::hint::black_box(s.round(2).expect("round"));
    });
    let pct_change = best(iters, || {
        std::hint::black_box(s.pct_change(1).expect("pct_change"));
    });
    let cumprod = best(iters, || {
        std::hint::black_box(s.cumprod().expect("cumprod"));
    });
    let cummin = best(iters, || {
        std::hint::black_box(s.cummin().expect("cummin"));
    });

    println!(
        "misc2 n={n}: idxmax={idxmax}ns idxmin={idxmin}ns nunique={nunique}ns abs={abs}ns round={round}ns pct_change={pct_change}ns cumprod={cumprod}ns cummin={cummin}ns"
    );
}
