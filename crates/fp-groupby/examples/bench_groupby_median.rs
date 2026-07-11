#![cfg_attr(test, feature(test))]

//! Bench + golden harness for the dense-Int64 groupby median fast path
//! (`try_groupby_median_dense_int64`).
//!
//! Usage:
//!   cargo run --release --example bench_groupby_median -- bench   # timing
//!   cargo run --release --example bench_groupby_median -- golden  # digest stream
//!
//! `golden` prints every output (key, value-bits) pair in result order so an
//! external `sha256sum` can certify the result is bit-identical before/after a
//! change. `bench` reports the min wall-time over repeated runs.

use std::time::Instant;

use fp_frame::Series;
use fp_groupby::{AggFunc, GroupByOptions, groupby_agg};
use fp_index::IndexLabel;
use fp_runtime::{EvidenceLedger, RuntimePolicy};
use fp_types::Scalar;

#[cfg(test)]
extern crate test;

/// Deterministic splitmix64 so the harness needs no rand dependency and is
/// reproducible across before/after builds.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

fn build_data(n: usize, num_groups: i64) -> (Series, Series) {
    let mut rng = Rng(0x1234_5678_9ABC_DEF0);
    let mut idx = Vec::with_capacity(n);
    let mut keys = Vec::with_capacity(n);
    let mut vals = Vec::with_capacity(n);
    for i in 0..n {
        idx.push(IndexLabel::Int64(i as i64));
        let k = (rng.next() % num_groups as u64) as i64;
        keys.push(Scalar::Int64(k));
        // Spread of finite floats; a few integral values to exercise ties.
        let v = (rng.next() % 100_000) as f64 * 0.5;
        vals.push(Scalar::Float64(v));
    }
    let key_series = Series::from_values("key", idx.clone(), keys).expect("keys");
    let val_series = Series::from_values("value", idx, vals).expect("vals");
    (key_series, val_series)
}

/// `cargo bench` entry point. The unstable libtest bencher reports the median
/// sample time, which makes this the decision surface for dispatch changes.
#[cfg(test)]
#[bench]
fn dense_int64_median_dispatch(b: &mut test::Bencher) {
    let (keys, vals) = build_data(2_000_000, 200);
    let policy = RuntimePolicy::strict();

    b.iter(|| {
        let mut ledger = EvidenceLedger::new();
        let out = groupby_agg(
            &keys,
            &vals,
            AggFunc::Median,
            GroupByOptions::default(),
            &policy,
            &mut ledger,
        )
        .expect("median");
        test::black_box(out);
    });
}

fn main() {
    let mode = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "bench".to_string());
    let n: usize = 2_000_000;
    let num_groups: i64 = 200;
    let (keys, vals) = build_data(n, num_groups);
    let policy = RuntimePolicy::strict();

    if mode == "golden" {
        let mut ledger = EvidenceLedger::new();
        let out = groupby_agg(
            &keys,
            &vals,
            AggFunc::Median,
            GroupByOptions::default(),
            &policy,
            &mut ledger,
        )
        .expect("median");
        for (label, val) in out.index().labels().iter().zip(out.values().iter()) {
            let bits = match val {
                Scalar::Float64(f) => f.to_bits(),
                _ => 0,
            };
            println!("{label:?}\t{bits:016x}");
        }
        return;
    }

    // bench
    let iters = 20;
    let mut best = f64::INFINITY;
    for _ in 0..iters {
        let mut ledger = EvidenceLedger::new();
        let t0 = Instant::now();
        let out = groupby_agg(
            &keys,
            &vals,
            AggFunc::Median,
            GroupByOptions::default(),
            &policy,
            &mut ledger,
        )
        .expect("median");
        let dt = t0.elapsed().as_secs_f64();
        std::hint::black_box(&out);
        if dt < best {
            best = dt;
        }
    }
    println!(
        "groupby median: n={n} groups={num_groups} best={:.3} ms",
        best * 1e3
    );
}
