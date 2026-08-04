//! Bench + golden for Column::unique on an all-valid contiguous-Utf8 column
//! (the kernel behind Series.unique on string data). n rows drawn from
//! `cardinality` distinct values. Golden = FNV-1a64 over the output strings.
//!
//! Run: cargo run -p fp-columnar --example bench_str_unique --release -- 200000 1000 200

use std::time::Instant;

use fp_columnar::Column;

fn build(n: usize, cardinality: usize) -> Column {
    let cardinality = cardinality.max(1);
    let mut bytes = Vec::with_capacity(n * 12);
    let mut offsets = Vec::with_capacity(n + 1);
    offsets.push(0usize);
    for row in 0..n {
        let mixed = (row as u64)
            .wrapping_mul(0x9E37_79B9_7F4A_7C15)
            .rotate_left(17)
            ^ (row as u64).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        let id = mixed % cardinality as u64;
        let s = format!("val_{id:06x}");
        bytes.extend_from_slice(s.as_bytes());
        offsets.push(bytes.len());
    }
    Column::from_utf8_contiguous(bytes, offsets)
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
    // Hash the output strings in order (length-prefixed) so order + content are
    // both pinned.
    let mut buf = Vec::new();
    let n = col.len();
    for i in 0..n {
        match col.value(i) {
            Some(fp_types::Scalar::Utf8(s)) => {
                buf.extend_from_slice(&(s.len() as u64).to_le_bytes());
                buf.extend_from_slice(s.as_bytes());
            }
            other => {
                buf.extend_from_slice(format!("{other:?}").as_bytes());
            }
        }
    }
    (fnv1a64(&buf), n)
}

fn main() {
    let mut args = std::env::args().skip(1);
    let n: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(200_000);
    let card: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(1000);
    let iters: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(200);

    let col = build(n, card);

    let out = col.unique().unwrap();
    let (d, len) = digest(&out);
    println!("GOLDEN n={n} card={card} out_len={len} fnv={d:016x}");

    // Warm (column reused — the OnceLock<Vec<Scalar>> cache is populated once).
    let _ = col.unique().unwrap();
    let t = Instant::now();
    let mut sink = 0usize;
    for _ in 0..iters {
        let out = col.unique().unwrap();
        sink = sink.wrapping_add(out.len());
    }
    let warm = t.elapsed();
    std::hint::black_box(sink);
    println!(
        "TIMING_WARM n={n} card={card} iters={iters} per_iter={:.4}ms",
        warm.as_secs_f64() * 1e3 / iters as f64
    );

    // Cold (fresh column each call — the realistic one-shot unique pattern).
    // We also time bare build() to subtract the common-mode construction cost.
    let cold_iters = iters.clamp(20, 100);
    let t = Instant::now();
    let mut bsink = 0usize;
    for _ in 0..cold_iters {
        let c = build(n, card);
        bsink = bsink.wrapping_add(c.len());
    }
    let build_only = t.elapsed();
    std::hint::black_box(bsink);

    let t = Instant::now();
    let mut sink = 0usize;
    for _ in 0..cold_iters {
        let c = build(n, card);
        let out = c.unique().unwrap();
        sink = sink.wrapping_add(out.len());
    }
    let build_plus_unique = t.elapsed();
    std::hint::black_box(sink);

    let unique_cold_ms =
        (build_plus_unique.as_secs_f64() - build_only.as_secs_f64()) * 1e3 / cold_iters as f64;
    println!(
        "TIMING_COLD n={n} card={card} iters={cold_iters} unique_only={unique_cold_ms:.4}ms (build={:.4}ms)",
        build_only.as_secs_f64() * 1e3 / cold_iters as f64
    );
}
