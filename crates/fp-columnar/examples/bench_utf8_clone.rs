//! Bench + golden digest for Column::clone on an all-valid contiguous-Utf8
//! column.
//!
//! Run: cargo run -p fp-columnar --example bench_utf8_clone --release
//!
//! A LazyContiguousUtf8 column's byte buffer is immutable after construction
//! (string ops always build a fresh buffer), so cloning can share the buffer
//! via Arc instead of deep-copying it — O(1) vs O(n). This is observationally
//! a deep copy because the data can never change underneath a shared reader.
//! Column::clone is the primitive behind Series.clone()/DataFrame.copy() and is
//! invoked all over alignment/concat/groupby paths.
//!
//! The golden proves the shared clone yields byte-identical observable values
//! to the original (and to the pre-Arc deep-copy path).

use std::hint::black_box;
use std::time::Instant;

use fp_columnar::Column;

/// Build an all-valid contiguous-Utf8 column of `n` rows, each `width` bytes,
/// with deterministic ASCII content.
fn build(n: usize, width: usize) -> Column {
    let mut bytes = Vec::with_capacity(n * width);
    let mut offsets = Vec::with_capacity(n + 1);
    offsets.push(0usize);
    for i in 0..n {
        for j in 0..width {
            // printable ASCII derived from (i, j) — varied but deterministic
            let b = b'a' + (((i.wrapping_mul(31).wrapping_add(j)) % 26) as u8);
            bytes.push(b);
        }
        offsets.push(bytes.len());
    }
    Column::from_utf8_contiguous(bytes, offsets)
}

fn fnv1a64_values(col: &Column) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for v in col.values() {
        // format is stable for Scalar::Utf8 content
        for b in format!("{v:?}").as_bytes() {
            h ^= *b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
        h ^= 0xff;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

fn main() {
    // Golden: a clone must observe byte-identical values to the original.
    let g = build(2000, 7);
    let g_clone = g.clone();
    let orig_h = fnv1a64_values(&g);
    let clone_h = fnv1a64_values(&g_clone);
    println!("GOLDEN_ORIG_FNV1A64   {orig_h:016x}");
    println!("GOLDEN_CLONE_FNV1A64  {clone_h:016x}");
    println!(
        "GOLDEN_MATCH {}",
        if orig_h == clone_h { "ok" } else { "MISMATCH" }
    );

    // Timing: clone a large column many times. The buffer is ~12 MB so the
    // pre-Arc deep copy is a 12 MB memcpy per clone; the Arc share is a pointer
    // bump. black_box prevents the loop being elided.
    let n: usize = 1_000_000;
    let width: usize = 12;
    let col = build(n, width);
    let iters = 500usize;

    // warm
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
        "TIMING clone n={n} width={width} iters={iters} total={:.3}ms per_clone={:.4}ms acc={acc}",
        d.as_secs_f64() * 1e3,
        d.as_secs_f64() * 1e3 / iters as f64
    );
}
