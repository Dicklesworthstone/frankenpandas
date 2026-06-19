//! value_counts on Float64 (~1000 distinct floored floats) — the case that falls to the
//! general ScalarKey path (values() materialize + FloatBits FxHash clustering). Candidate
//! for a typed f64-bits + splitmix path like unique/mode.
//!
//! Run: cargo run -p fp-frame --example bench_vcf --release -- 2000000 30

use std::time::Instant;

use fp_frame::Series;
use fp_index::IndexLabel;
use fp_types::Scalar;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let n: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(2_000_000);
    let iters: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(30);
    let labels: Vec<IndexLabel> = (0..n as i64).map(IndexLabel::Int64).collect();
    let mut st: u64 = 0xabcd_1234_5678_9f01;
    let mut nextf = || {
        st = st.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        ((st >> 11) as f64) / (1u64 << 53) as f64
    };
    let vals: Vec<Scalar> = (0..n).map(|_| Scalar::Float64((nextf() * 1000.0).floor())).collect();
    let s = Series::from_values("v", labels, vals).unwrap();

    let mut best = u128::MAX;
    for _ in 0..iters {
        let t = Instant::now();
        let out = s.value_counts().expect("value_counts");
        let e = t.elapsed().as_nanos();
        std::hint::black_box(&out);
        if e < best {
            best = e;
        }
    }
    println!("value_counts_f64 n={n} iters={iters}: best={best}ns");
}
