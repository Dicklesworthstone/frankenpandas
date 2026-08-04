//! Bench + byte-digest for DataFrame to_csv on an all-numeric frame
//! (br-frankenpandas-csvw). The writer is row-major: per cell it materializes a
//! Scalar (OnceLock Vec<Scalar> per lazy column) + allocates a String. An
//! all-numeric fast path can read the typed f64/i64 slices and format with the
//! same `v.to_string()` directly into the output buffer. The FNV digest of the
//! full output proves byte-identity before/after.
//!
//! Run: cargo run -p fp-conformance --example bench_csv_write --release

use std::{collections::BTreeMap, time::Instant};

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::{Index, IndexLabel};
use fp_io::write_csv_string;

fn fnv1a64(s: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

fn build(n: usize) -> DataFrame {
    let labels: Vec<IndexLabel> = (0..n as i64).map(IndexLabel::Int64).collect();
    let index = Index::new(labels);
    let mut cols = BTreeMap::new();
    let mut order = Vec::new();
    for c in 0..4 {
        let v: Vec<f64> = (0..n)
            .map(|i| ((i as u64).wrapping_mul(2654435761 + c) % 100000) as f64 * 0.125)
            .collect();
        let name = format!("f{c}");
        cols.insert(name.clone(), Column::from_f64_values(v));
        order.push(name);
    }
    for c in 0..2 {
        let v: Vec<i64> = (0..n as i64)
            .map(|i| i.wrapping_mul(31 + c) % 9973)
            .collect();
        let name = format!("i{c}");
        cols.insert(name.clone(), Column::from_i64_values(v));
        order.push(name);
    }
    // One all-valid contiguous Utf8 column, with values that exercise CSV
    // QUOTE_MINIMAL (plain, comma, embedded quote, newline) so the byte-identity
    // check covers the quoting path.
    let samples = ["plain", "a,b", "he said \"hi\"", "line1\nline2", "x"];
    let mut bytes = Vec::<u8>::new();
    let mut offsets = vec![0usize];
    for i in 0..n {
        bytes.extend_from_slice(samples[i % samples.len()].as_bytes());
        offsets.push(bytes.len());
    }
    cols.insert(
        "s".to_string(),
        Column::from_utf8_contiguous(bytes, offsets),
    );
    order.push("s".to_string());
    DataFrame::new_with_column_order(index, cols, order).expect("frame")
}

fn main() {
    let n: usize = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(500_000);
    let iters: usize = std::env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(10);

    // Digest on a small frame (stable, easy to eyeball) + the large one.
    let small = build(20);
    let s = write_csv_string(&small).expect("csv");
    println!("SMALL_FNV {:016x} len={}", fnv1a64(&s), s.len());
    // show the first line's terminator bytes
    let head: String = s
        .chars()
        .take(40)
        .map(|c| {
            if c == '\n' {
                '|'
            } else if c == '\r' {
                '^'
            } else {
                c
            }
        })
        .collect();
    println!("HEAD {head}");

    let frame = build(n);
    let big = write_csv_string(&frame).expect("csv");
    println!("BIG_FNV {:016x} len={}", fnv1a64(&big), big.len());

    let t = Instant::now();
    let mut sink = 0usize;
    for _ in 0..iters {
        let out = write_csv_string(&frame).expect("csv");
        sink = sink.wrapping_add(out.len());
    }
    let d = t.elapsed();
    println!(
        "TIMING n={n} iters={iters} per_iter={:.3}ms sink={sink}",
        d.as_secs_f64() * 1e3 / iters as f64
    );
}
