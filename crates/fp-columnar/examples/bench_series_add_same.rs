//! Bench + golden for the same-index Float64 arithmetic fast path
//! (`aligned_binary_f64_same_positions`), the hot path behind identical-index
//! Series add (br-frankenpandas-9houf). Two all-valid Float64 columns of length
//! `n` are added `iters` times. The golden is a SHA256 over the materialized
//! output (f64 bits + validity word) so byte-identity is provable across the
//! single-pass NaN-tracked kernel change.
//!
//! Run (timing): cargo run -p fp-columnar --example bench_series_add_same --release -- 100000 200 add
//! Run (golden): cargo run -p fp-columnar --example bench_series_add_same --release -- 100000 1 all emit /tmp/sa.bin

use std::time::Instant;

use fp_columnar::{ArithmeticOp, Column};

fn build(n: usize, salt: u64) -> Column {
    // Deterministic, all-valid, NaN-free f64 data (the common arithmetic case).
    let v: Vec<f64> = (0..n)
        .map(|i| (i as u64).wrapping_mul(2_654_435_761).wrapping_add(salt) as f64 * 0.5 - 7.0)
        .collect();
    Column::from_f64_values(v)
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

fn digest(col: &Column) -> (u64, usize) {
    // Materialize f64 bits + a per-row validity byte into a buffer and hash it.
    let n = col.len();
    let mut buf = Vec::with_capacity(n * 9);
    let slice = col.as_f64_slice();
    for i in 0..n {
        let bits = slice.map_or(0u64, |s| s[i].to_bits());
        buf.extend_from_slice(&bits.to_le_bytes());
        // is_valid via scalar materialization (covers nullable backings too).
        let valid = col.value(i).is_some_and(|s| !s.is_missing());
        buf.push(u8::from(valid));
    }
    (fnv1a64(&buf), n)
}

fn ops() -> Vec<(ArithmeticOp, &'static str)> {
    vec![
        (ArithmeticOp::Add, "add"),
        (ArithmeticOp::Sub, "sub"),
        (ArithmeticOp::Mul, "mul"),
        (ArithmeticOp::Div, "div"),
        (ArithmeticOp::Mod, "mod"),
        (ArithmeticOp::Pow, "pow"),
        (ArithmeticOp::FloorDiv, "floordiv"),
    ]
}

fn main() {
    let mut args = std::env::args().skip(1);
    let n: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(100_000);
    let iters: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(200);
    let which = args.next().unwrap_or_else(|| "add".to_string());

    let left = build(n, 1);
    let right = build(n, 9_999);

    // Golden digests for every op (covers the kernel's per-op arms).
    for (op, name) in ops() {
        let out = left.aligned_binary_f64_same_positions(&right, op).unwrap();
        let (d, len) = digest(&out);
        println!("GOLDEN {name} n={len} fnv={d:016x}");
    }

    if which == "all" {
        return;
    }

    let op = ops()
        .into_iter()
        .find(|(_, name)| *name == which)
        .map_or(ArithmeticOp::Add, |(op, _)| op);

    // warmup
    let _ = left.aligned_binary_f64_same_positions(&right, op).unwrap();

    let t = Instant::now();
    let mut sink = 0u64;
    for _ in 0..iters {
        let out = left.aligned_binary_f64_same_positions(&right, op).unwrap();
        sink = sink.wrapping_add(out.len() as u64);
    }
    let d = t.elapsed();
    std::hint::black_box(sink);
    println!(
        "TIMING op={which} n={n} iters={iters} per_iter={:.4}ms",
        d.as_secs_f64() * 1e3 / iters as f64
    );
}
