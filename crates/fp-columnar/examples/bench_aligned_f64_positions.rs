//! Bench + golden for the position-based `Column::aligned_binary_f64`, the
//! different-index/reindex Float64 alignment path behind cross-index Series add
//! (br-frankenpandas-419of). Two all-valid Float64 columns of length `n` are
//! combined through explicit `Some(k)` position vectors `iters` times. A
//! "gappy" variant (every 7th right slot `None`) exercises the
//! invalid/null-introducing branch so the golden covers both.
//!
//! Run (timing): cargo run -p fp-columnar --example bench_aligned_f64_positions --release -- 100000 200 add
//! Run (golden): cargo run -p fp-columnar --example bench_aligned_f64_positions --release -- 100000 1 all

use std::time::Instant;

use fp_columnar::{ArithmeticOp, Column};

fn build(n: usize, salt: u64) -> Column {
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

fn digest(col: &Column) -> u64 {
    let n = col.len();
    let mut buf = Vec::with_capacity(n * 9);
    let slice = col.as_f64_slice();
    for i in 0..n {
        let bits = slice.map_or_else(
            || col.value(i).and_then(fp_scalar_f64_bits).unwrap_or(0),
            |s| s[i].to_bits(),
        );
        buf.extend_from_slice(&bits.to_le_bytes());
        let valid = col.value(i).is_some_and(|s| !s.is_missing());
        buf.push(u8::from(valid));
    }
    fnv1a64(&buf)
}

fn fp_scalar_f64_bits(s: &fp_types::Scalar) -> Option<u64> {
    match s {
        fp_types::Scalar::Float64(f) => Some(f.to_bits()),
        _ => None,
    }
}

fn ops() -> [(ArithmeticOp, &'static str); 7] {
    [
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

    // Full 1:1 alignment (the common all-overlap reindex case).
    let full: Vec<Option<usize>> = (0..n).map(Some).collect();
    // Gappy: every 7th right position missing -> null-introducing branch.
    let gappy_r: Vec<Option<usize>> = (0..n)
        .map(|k| if k % 7 == 0 { None } else { Some(k) })
        .collect();

    for (op, name) in ops() {
        let out_full = left.aligned_binary_f64(&right, &full, &full, op).unwrap();
        let out_gap = left
            .aligned_binary_f64(&right, &full, &gappy_r, op)
            .unwrap();
        println!(
            "GOLDEN {name} full={:016x} gappy={:016x}",
            digest(&out_full),
            digest(&out_gap)
        );
    }

    if which == "all" {
        return;
    }

    let op = ops()
        .into_iter()
        .find(|(_, name)| *name == which)
        .map_or(ArithmeticOp::Add, |(op, _)| op);

    let _ = left.aligned_binary_f64(&right, &full, &full, op).unwrap();

    let t = Instant::now();
    let mut sink = 0u64;
    for _ in 0..iters {
        let out = left.aligned_binary_f64(&right, &full, &full, op).unwrap();
        sink = sink.wrapping_add(out.len() as u64);
    }
    let d = t.elapsed();
    std::hint::black_box(sink);
    println!(
        "TIMING op={which} n={n} iters={iters} per_iter={:.4}ms",
        d.as_secs_f64() * 1e3 / iters as f64
    );
}
