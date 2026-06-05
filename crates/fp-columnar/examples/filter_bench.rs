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

use fp_columnar::{ArithmeticOp, Column, ComparisonOp};
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

fn build_other(n: usize) -> Column {
    // Same length as build_column(n) but a distinct (negated-scramble) value set.
    let values: Vec<Scalar> = (0..n as i64)
        .map(|i| {
            let h = (i as u64).wrapping_mul(0xff51_afd7_ed55_8ccd);
            Scalar::Float64(1.0 - ((h >> 11) as f64) / (1u64 << 52) as f64)
        })
        .collect();
    Column::new(DType::Float64, values).expect("f64 other column")
}

fn run_where(col: &Column, other: &Column) -> Column {
    // mask = col > 0; col.where(mask, other) — typed branchless select.
    let cond = col
        .compare_scalar(&Scalar::Float64(0.0), ComparisonOp::Gt)
        .expect("compare");
    col.where_cond_series(&cond, other).expect("where")
}

fn run_clip(col: &Column) -> Column {
    col.clip(Some(-0.5), Some(0.5)).expect("clip")
}

fn build_nullable(n: usize) -> Column {
    // Aligned add with ~10% left-gaps -> a LazyNullableFloat64 column (nulls
    // where the left side is unmatched).
    let left = build_column(n);
    let right = build_other(n);
    let lp: Vec<Option<usize>> = (0..n)
        .map(|i| if i % 10 == 0 { None } else { Some(i) })
        .collect();
    let rp: Vec<Option<usize>> = (0..n).map(Some).collect();
    left.aligned_binary_f64(&right, &lp, &rp, ArithmeticOp::Add)
        .expect("aligned add")
}

fn run_ntake(col: &Column) -> Column {
    // Gather every even row (a 50%-selectivity positional take, as iloc/sort do).
    let pos: Vec<usize> = (0..col.len()).filter(|i| i % 2 == 0).collect();
    col.take_positions(&pos)
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
    if args.get(1).map(String::as_str) == Some("golden_where") {
        let n: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(5_000);
        let col = build_column(n);
        let other = build_other(n);
        let out = run_where(&col, &other);
        println!(
            "where_golden n={n} out_len={} digest={:016x}",
            out.len(),
            digest(&out)
        );
        return;
    }
    if args.get(1).map(String::as_str) == Some("where") {
        let n: usize = args
            .get(2)
            .and_then(|s| s.parse().ok())
            .unwrap_or(2_000_000);
        let iters: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(200);
        let col = build_column(n);
        let other = build_other(n);
        let start = Instant::now();
        let mut sink: usize = 0;
        for _ in 0..iters {
            let out = run_where(&col, &other);
            sink = sink.wrapping_add(out.len());
        }
        let elapsed = start.elapsed();
        eprintln!(
            "where_bench n={n} iters={iters} {:.3}s ({:.3} ms/iter), sink={sink}",
            elapsed.as_secs_f64(),
            elapsed.as_secs_f64() * 1000.0 / iters as f64,
        );
        return;
    }
    if args.get(1).map(String::as_str) == Some("golden_clip") {
        let n: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(5_000);
        let out = run_clip(&build_column(n));
        println!(
            "clip_golden n={n} out_len={} digest={:016x}",
            out.len(),
            digest(&out)
        );
        return;
    }
    if args.get(1).map(String::as_str) == Some("golden_ntake") {
        let n: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(5_000);
        let out = run_ntake(&build_nullable(n));
        println!(
            "ntake_golden n={n} out_len={} digest={:016x}",
            out.len(),
            digest(&out)
        );
        return;
    }
    if args.get(1).map(String::as_str) == Some("ntake") {
        let n: usize = args
            .get(2)
            .and_then(|s| s.parse().ok())
            .unwrap_or(2_000_000);
        let iters: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(200);
        let col = build_nullable(n);
        let start = Instant::now();
        let mut sink: usize = 0;
        for _ in 0..iters {
            let out = run_ntake(&col);
            sink = sink.wrapping_add(out.len());
        }
        let elapsed = start.elapsed();
        eprintln!(
            "ntake_bench n={n} iters={iters} {:.3}s ({:.3} ms/iter), sink={sink}",
            elapsed.as_secs_f64(),
            elapsed.as_secs_f64() * 1000.0 / iters as f64,
        );
        return;
    }
    if args.get(1).map(String::as_str) == Some("clip") {
        let n: usize = args
            .get(2)
            .and_then(|s| s.parse().ok())
            .unwrap_or(2_000_000);
        let iters: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(200);
        let col = build_column(n);
        let start = Instant::now();
        let mut sink: usize = 0;
        for _ in 0..iters {
            let out = run_clip(&col);
            sink = sink.wrapping_add(out.len());
        }
        let elapsed = start.elapsed();
        eprintln!(
            "clip_bench n={n} iters={iters} {:.3}s ({:.3} ms/iter), sink={sink}",
            elapsed.as_secs_f64(),
            elapsed.as_secs_f64() * 1000.0 / iters as f64,
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
