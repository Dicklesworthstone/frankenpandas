//! Bench + golden for Column::clone on all-valid typed numeric columns.
//!
//! Run: cargo run -p fp-columnar --example bench_numeric_clone --release
//!
//! LazyAllValid{Int64,Float64,Bool} buffers are immutable after construction,
//! so cloning shares the buffer via Arc (O(1)) instead of deep-copying it.
//! Column::clone of typed numeric columns is hot in groupby/concat/align/copy
//! (cf. the 8.7x groupby win from preserving typed backing across clone).
//! Golden proves a clone observes byte-identical values to the original.

use std::hint::black_box;
use std::time::Instant;

use fp_columnar::Column;

fn fnv1a64(col: &Column) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for v in col.values() {
        for b in format!("{v:?}").as_bytes() {
            h ^= *b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
        h ^= 0xff;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

fn bench(label: &str, col: &Column, iters: usize) {
    black_box(col.clone());
    let t = Instant::now();
    let mut acc = 0usize;
    for _ in 0..iters {
        let c = black_box(col.clone());
        acc = acc.wrapping_add(c.len());
        black_box(&c);
    }
    let d = t.elapsed();
    println!(
        "TIMING {label} n={} iters={iters} total={:.3}ms per_clone={:.4}ms acc={acc}",
        col.len(),
        d.as_secs_f64() * 1e3,
        d.as_secs_f64() * 1e3 / iters as f64
    );
}

fn main() {
    let n: usize = 2_000_000;

    // Golden: clone observes identical values (small columns).
    let gi = Column::from_i64_values((0..2000i64).map(|i| i.wrapping_mul(7)).collect());
    let gf = Column::from_f64_values((0..2000).map(|i| i as f64 * 0.5).collect());
    for (name, c) in [("i64", &gi), ("f64", &gf)] {
        let h0 = fnv1a64(c);
        let h1 = fnv1a64(&c.clone());
        println!(
            "GOLDEN {name} orig={h0:016x} clone={h1:016x} {}",
            if h0 == h1 { "ok" } else { "MISMATCH" }
        );
    }

    let icol = Column::from_i64_values((0..n as i64).map(|i| i.wrapping_mul(2654435761)).collect());
    let fcol = Column::from_f64_values((0..n).map(|i| (i as f64).sqrt()).collect());
    bench("i64_clone", &icol, 500);
    bench("f64_clone", &fcol, 500);
}
