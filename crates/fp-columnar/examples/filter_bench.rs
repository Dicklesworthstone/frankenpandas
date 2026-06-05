//! Isolates the realistic boolean-filter pipeline `mask = col > t; col[mask]`
//! on a typed column: `compare_scalar` (already typed) produces a *lazy* Bool
//! mask, then `filter_by_mask` selects. Before this lever filter_by_mask read
//! the mask through `mask.values` — forcing the lazy Bool mask to materialize a
//! full Vec<Scalar::Bool> on every filter; now it reads the contiguous `bool`
//! buffer via `as_bool_slice`.
//!
//! Modes:
//!   filter_bench golden <n>      -> deterministic output digest (sha proof)
//!   filter_bench <n> <iters>     -> timed compare+filter loop (hyperfine target)

use std::time::Instant;

use fp_columnar::{Column, ComparisonOp};
use fp_types::{DType, Scalar};

fn build_column(n: usize) -> Column {
    // Scrambled values straddling 0 so `> 0` keeps ~50% of rows.
    let values: Vec<Scalar> = (0..n as i64)
        .map(|i| {
            let h = (i as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15);
            Scalar::Float64(((h >> 11) as f64) / (1u64 << 52) as f64 - 1.0)
        })
        .collect();
    Column::new(DType::Float64, values).expect("f64 column")
}

fn run_filter(col: &Column) -> Column {
    // Threshold ~0 keeps roughly half the rows (the scrambled values straddle 0).
    let mask = col
        .compare_scalar(&Scalar::Float64(0.0), ComparisonOp::Gt)
        .expect("compare");
    col.filter_by_mask(&mask).expect("filter")
}

fn digest(col: &Column) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut mix = |x: u64| {
        h ^= x;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    };
    mix(col.len() as u64);
    for v in col.values() {
        match v {
            Scalar::Float64(f) => mix(f.to_bits()),
            Scalar::Int64(i) => mix(*i as u64),
            Scalar::Null(_) => mix(0xDEAD_BEEF),
            other => mix(format!("{other:?}")
                .bytes()
                .fold(0u64, |a, b| a.wrapping_mul(131).wrapping_add(u64::from(b)))),
        }
    }
    h
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("golden") {
        let n: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(5_000);
        let col = build_column(n);
        let out = run_filter(&col);
        println!(
            "filter_golden n={n} out_len={} digest={:016x}",
            out.len(),
            digest(&out)
        );
        return;
    }

    let n: usize = args
        .get(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(2_000_000);
    let iters: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(200);
    let col = build_column(n);

    let start = Instant::now();
    let mut sink: usize = 0;
    for _ in 0..iters {
        let out = run_filter(&col);
        sink = sink.wrapping_add(out.len());
    }
    let elapsed = start.elapsed();
    eprintln!(
        "filter_bench n={n} iters={iters} {:.3}s ({:.3} ms/iter), sink={sink}",
        elapsed.as_secs_f64(),
        elapsed.as_secs_f64() * 1000.0 / iters as f64,
    );
}
