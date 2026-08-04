//! Bench + golden for DataFrame/Series to_numpy typed fast path
//! (br-frankenpandas-tonp1). The old DataFrame::to_numpy did a BTreeMap lookup +
//! column.values() per cell (n*m times); the fast path resolves columns once and
//! reads contiguous typed slices. Golden FNV over the output proves identity.
//!
//! Run: cargo run -p fp-frame --example bench_to_numpy --release

use std::{collections::BTreeMap, hint::black_box, time::Instant};

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::{Index, IndexLabel};

fn fnv_f64(rows: &[Vec<f64>]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for r in rows {
        for v in r {
            for b in v.to_bits().to_le_bytes() {
                h ^= b as u64;
                h = h.wrapping_mul(0x100000001b3);
            }
        }
    }
    h
}

fn build(n: usize) -> DataFrame {
    let index = Index::new((0..n as i64).map(IndexLabel::Int64).collect());
    let mut cols = BTreeMap::new();
    let mut order = Vec::new();
    for c in 0..6 {
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
        .unwrap_or(20);

    let small = build(37);
    println!("GOLDEN_FNV {:016x}", fnv_f64(&small.to_numpy()));

    let frame = build(n);
    black_box(frame.to_numpy());
    let t = Instant::now();
    let mut sink = 0usize;
    for _ in 0..iters {
        let m = black_box(frame.to_numpy());
        sink = sink.wrapping_add(m.len());
    }
    let d = t.elapsed();
    println!(
        "TIMING n={n} iters={iters} per_iter={:.3}ms sink={sink}",
        d.as_secs_f64() * 1e3 / iters as f64
    );
}
