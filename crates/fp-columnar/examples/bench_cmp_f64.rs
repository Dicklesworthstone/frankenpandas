//! Bench + golden for numeric element-wise comparison (`Column::gt/lt/ge/le/
//! eq/ne` column-vs-column and `compare_scalar` against a midpoint scalar),
//! the Bool-producing path behind `Series` relational ops on numeric data.
//! Two all-valid Float64 columns of length `n` are compared `iters` times. A
//! "gappy" right column (every 11th slot missing) exercises the
//! missing-propagation branch so the golden covers valid + null outputs.
//!
//! Run (timing): cargo run -p fp-columnar --example bench_cmp_f64 --release -- 100000 200 gt
//! Run (nullable Int64 scalar timing): cargo run -p fp-columnar --example bench_cmp_f64 --release -- 5000000 9 i64-nullable
//! Run (golden): cargo run -p fp-columnar --example bench_cmp_f64 --release -- 100000 1 all

use std::time::Instant;

use fp_columnar::{Column, ComparisonOp};
use fp_types::{DType, NullKind, Scalar};

/// Bounded, exactly-f64-representable pseudo-random value at row `i`. Distinct
/// `(mult, add)` seeds give independently distributed columns, so left-vs-right
/// comparisons land a realistic ~50/50 true/false mix (not a one-sided scan
/// the branch predictor trivially folds, and small enough that the u64 -> f64
/// cast is lossless).
fn value_at(i: usize, mult: u64, add: u64) -> f64 {
    let h = (i as u64).wrapping_mul(mult).wrapping_add(add);
    (h % 100_003) as f64 * 0.5 - 25_000.0
}

fn build(n: usize, mult: u64, add: u64) -> Column {
    let v: Vec<f64> = (0..n).map(|i| value_at(i, mult, add)).collect();
    Column::from_f64_values(v)
}

/// Same distribution as `build` but every 11th slot is missing, so comparisons
/// hit the null-propagation branch (missing in -> missing out).
fn build_gappy(n: usize, mult: u64, add: u64) -> Column {
    let values: Vec<Scalar> = (0..n)
        .map(|i| {
            if i % 11 == 0 {
                Scalar::Null(NullKind::Null)
            } else {
                Scalar::Float64(value_at(i, mult, add))
            }
        })
        .collect();
    Column::new(DType::Float64, values).expect("gappy float column")
}

/// Integer sibling of `build_gappy`: every 11th slot is missing and the
/// remaining values span both sides of the scalar probe.
fn build_gappy_i64(n: usize, mult: u64, add: u64) -> Column {
    let values: Vec<Scalar> = (0..n)
        .map(|i| {
            if i % 11 == 0 {
                Scalar::Null(NullKind::Null)
            } else {
                let h = (i as u64).wrapping_mul(mult).wrapping_add(add);
                Scalar::Int64((h % 100_003) as i64 - 50_001)
            }
        })
        .collect();
    Column::new(DType::Int64, values).expect("gappy integer column")
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// Tri-state digest of a Bool column: 0 = false, 1 = true, 2 = missing.
fn digest_bool(col: &Column) -> u64 {
    let n = col.len();
    let mut buf = Vec::with_capacity(n);
    if let Some(slice) = col.as_bool_slice() {
        for &b in slice {
            buf.push(u8::from(b));
        }
    } else {
        for i in 0..n {
            let code = match col.value(i) {
                Some(s) if s.is_missing() => 2u8,
                Some(Scalar::Bool(b)) => u8::from(*b),
                _ => 3u8,
            };
            buf.push(code);
        }
    }
    fnv1a64(&buf)
}

fn ops() -> [(ComparisonOp, &'static str); 6] {
    [
        (ComparisonOp::Gt, "gt"),
        (ComparisonOp::Lt, "lt"),
        (ComparisonOp::Ge, "ge"),
        (ComparisonOp::Le, "le"),
        (ComparisonOp::Eq, "eq"),
        (ComparisonOp::Ne, "ne"),
    ]
}

fn apply(left: &Column, right: &Column, op: ComparisonOp) -> Column {
    match op {
        ComparisonOp::Gt => left.gt(right),
        ComparisonOp::Lt => left.lt(right),
        ComparisonOp::Ge => left.ge(right),
        ComparisonOp::Le => left.le(right),
        ComparisonOp::Eq => left.eq(right),
        ComparisonOp::Ne => left.ne(right),
    }
    .expect("comparison")
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let n: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(100_000);
    let iters: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(200);
    let which = args.get(3).map(String::as_str).unwrap_or("gt");

    let left = build(n, 2_654_435_761, 0);
    let right = build(n, 40_503, 12_345);
    let gappy = build_gappy(n, 40_503, 12_345);
    let midpoint = Scalar::Float64(0.0);

    if which == "i64-nullable" {
        let gappy_i64 = build_gappy_i64(n, 2_654_435_761, 12_345);
        let midpoint_i64 = Scalar::Int64(0);
        let start = Instant::now();
        let mut checksum: u64 = 0;
        for _ in 0..iters {
            let out = gappy_i64
                .compare_scalar(&midpoint_i64, ComparisonOp::Gt)
                .expect("nullable Int64 scalar comparison");
            checksum = checksum.wrapping_add(out.len() as u64);
            std::hint::black_box(out);
        }
        let elapsed = start.elapsed();
        let per_iter_us = elapsed.as_secs_f64() * 1e6 / iters as f64;
        println!(
            "cmp_i64 scalar-nullable n={n} iters={iters} total={:.3}ms per_iter={per_iter_us:.2}us checksum={checksum}",
            elapsed.as_secs_f64() * 1e3,
        );
        return;
    }

    if which == "all" {
        // Golden: digest column-vs-column, scalar, and gappy-null outputs.
        for (op, name) in ops() {
            let cc = apply(&left, &right, op);
            let sc = left.compare_scalar(&midpoint, op).expect("compare_scalar");
            let gp = apply(&left, &gappy, op);
            println!(
                "cmp_f64 {name} n={n} cc={:016x} scalar={:016x} gappy={:016x}",
                digest_bool(&cc),
                digest_bool(&sc),
                digest_bool(&gp),
            );
        }
        return;
    }

    let op = ops()
        .into_iter()
        .find(|(_, name)| *name == which)
        .map(|(op, _)| op)
        .unwrap_or(ComparisonOp::Gt);

    // Timing loop. `checksum` keeps the result observable so the compare is
    // not optimized away.
    let start = Instant::now();
    let mut checksum: u64 = 0;
    for _ in 0..iters {
        let out = apply(&left, &right, op);
        if let Some(slice) = out.as_bool_slice() {
            checksum = checksum.wrapping_add(slice.iter().filter(|&&b| b).count() as u64);
        } else {
            checksum = checksum.wrapping_add(digest_bool(&out));
        }
    }
    let elapsed = start.elapsed();
    let per_iter_us = elapsed.as_secs_f64() * 1e6 / iters as f64;
    println!(
        "cmp_f64 {which} n={n} iters={iters} total={:.3}ms per_iter={per_iter_us:.2}us checksum={checksum}",
        elapsed.as_secs_f64() * 1e3,
    );
}
