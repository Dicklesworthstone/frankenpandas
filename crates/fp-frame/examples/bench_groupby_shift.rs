//! Bench + golden digest for SeriesGroupBy::shift (br-frankenpandas-gbcum).
//!
//! Run: cargo run -p fp-frame --example bench_groupby_shift --release -- <n> <iters>
//!
//! The dense path (dense gid + per-gid ring buffer) replaces the generic
//! build_groups + per-group Vec<Scalar> shift. `FP_NO_GBSHIFT=1` forces the
//! generic path for the baseline; the printed `chk` is an order-sensitive FNV
//! digest of the full output (incl. group-boundary nulls) so the two runs prove
//! bit-identicality.

use std::{hint::black_box, time::Instant};

use fp_columnar::Column;
use fp_frame::Series;
use fp_index::{Index, IndexLabel};
use fp_types::Scalar;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let n: usize = args
        .get(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(1_000_000);
    let iters: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(30);
    let groups = 100usize;

    let labels: Vec<IndexLabel> = (0..n as i64).map(IndexLabel::Int64).collect();
    let keys: Vec<i64> = (0..n).map(|i| (i % groups) as i64).collect();
    let vals: Vec<f64> = (0..n)
        .map(|i| (i.wrapping_mul(37) % 9973) as f64 * 0.25)
        .collect();
    let value = Series::new(
        "v".to_string(),
        Index::new(labels.clone()),
        Column::from_f64_values(vals),
    )
    .unwrap();
    let key = Series::new(
        "k".to_string(),
        Index::new(labels),
        Column::from_i64_values(keys),
    )
    .unwrap();

    let out = value.groupby(&key).unwrap().shift(1).unwrap();
    let mut chk: u64 = 0xcbf29ce484222325;
    for v in out.values() {
        let bits = match v {
            Scalar::Float64(f) => f.to_bits(),
            _ => 0xDEAD_BEEF_DEAD_BEEF,
        };
        chk = (chk ^ bits).wrapping_mul(0x100000001b3);
    }

    let mut sink = 0usize;
    let start = Instant::now();
    for _ in 0..iters {
        let s = black_box(value.groupby(&key).unwrap().shift(1).unwrap());
        sink ^= s.len();
    }
    let elapsed = start.elapsed();
    println!(
        "groupby_shift n={n} iters={iters}: {:.3} ms/iter (chk={chk:016x} sink={sink})",
        elapsed.as_secs_f64() * 1000.0 / iters as f64
    );
}
