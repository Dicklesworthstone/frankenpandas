//! Bench + golden digest for DataFrame::dropna (row-wise, axis=0).
//!
//! Run: cargo run -p fp-frame --example bench_dropna --release -- <n> <iters>
//!
//! `chk` is an FNV digest of the kept index labels (a stable golden for the
//! kept-row set); `kept` is the surviving row count.
//!
//! Profiling note (br-frankenpandas-dropnatyped, REJECTED): a typed per-row
//! missing-count fast path (skip all-valid Int64/Utf8/Bool O(1), NaN-scan
//! Float64 via as_f64_slice — avoiding the per-column Vec<Scalar>
//! materialization) measured only ~1.03-1.08x here, cold or warm. dropna is
//! GATHER-bound: the cost is filter_rows building the kept-row output frame
//! (take_positions per column, already typed), not the missing detection.

use std::{collections::BTreeMap, hint::black_box, time::Instant};

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::{Index, IndexLabel};

fn build(n: usize) -> DataFrame {
    let labels: Vec<IndexLabel> = (0..n as i64).map(IndexLabel::Int64).collect();
    let mut map = BTreeMap::new();
    let mut order = Vec::new();
    for c in 0..6 {
        let name = format!("i{c}");
        map.insert(
            name.clone(),
            Column::from_i64_values((0..n as i64).collect()),
        );
        order.push(name);
    }
    let name = "s".to_string();
    map.insert(
        name.clone(),
        Column::from_i64_values((0..n as i64).map(|x| x % 100).collect()),
    );
    order.push(name);
    for c in 0..3usize {
        let name = format!("f{c}");
        let vals: Vec<f64> = (0..n)
            .map(|i| {
                if (i.wrapping_mul(c + 7)) % 23 == 0 {
                    f64::NAN
                } else {
                    (i % 9973) as f64 * 0.5
                }
            })
            .collect();
        map.insert(name.clone(), Column::from_f64_values(vals));
        order.push(name);
    }
    DataFrame::new_with_column_order(Index::new(labels), map, order).expect("frame")
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let n: usize = args
        .get(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(1_000_000);
    let iters: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(30);
    let df = build(n);

    let out = df.dropna().expect("dropna");
    let mut chk: u64 = 0xcbf29ce484222325;
    for lbl in out.index().labels() {
        let bits = match lbl {
            IndexLabel::Int64(v) => *v as u64,
            _ => 0xDEAD,
        };
        chk = (chk ^ bits).wrapping_mul(0x100000001b3);
    }

    let mut sink = 0usize;
    let start = Instant::now();
    for _ in 0..iters {
        sink ^= black_box(df.dropna().expect("dropna")).len();
    }
    let elapsed = start.elapsed();
    println!(
        "dropna n={n} iters={iters}: {:.3} ms/iter (kept={} chk={chk:016x} sink={sink})",
        elapsed.as_secs_f64() * 1000.0 / iters as f64,
        out.len(),
    );
}
