//! Write-only bench + golden for `write_csv_string` on the quarter-affine
//! all-Float64 frame used by the high-ram CSV roundtrip profile
//! (br-frankenpandas-uza04.84). Each column `io_col_{c}` holds
//! `row * 1.25 + c`, exactly `build_numeric_frame` in
//! `high_ram_perf_baseline.rs`, so every column is quarter-affine and routes
//! through the typed quarter-plan writer whose integer Display residual this
//! bead attacks.
//!
//! Run (timing):  cargo run -p fp-io --example bench_csv_quarter --release -- 100000 20 50
//! Run (golden):  cargo run -p fp-io --example bench_csv_quarter --release -- 100000 20 1 emit /tmp/q.csv
//!                then `sha256sum /tmp/q.csv`.

use std::{collections::BTreeMap, time::Instant};

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::{Index, IndexLabel};
use fp_io::write_csv_string;

fn build(rows: usize, cols: usize) -> DataFrame {
    let labels: Vec<IndexLabel> = (0..rows as i64).map(IndexLabel::Int64).collect();
    let index = Index::new(labels);
    let mut map = BTreeMap::new();
    let mut order = Vec::with_capacity(cols);
    for c in 0..cols {
        let v: Vec<f64> = (0..rows)
            .map(|row| (row as f64 * 1.25) + c as f64)
            .collect();
        let name = format!("io_col_{c}");
        map.insert(name.clone(), Column::from_f64_values(v));
        order.push(name);
    }
    DataFrame::new_with_column_order(index, map, order).expect("frame")
}

fn main() {
    let mut args = std::env::args().skip(1);
    let rows: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(100_000);
    let cols: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(20);
    let iters: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(50);
    let emit = args.next();
    let emit_path = args.next();

    let frame = build(rows, cols);

    // Warmup (also the byte source for the optional golden emit).
    let warm = write_csv_string(&frame).expect("csv");
    println!("LEN {} rows={rows} cols={cols}", warm.len());

    if emit.as_deref() == Some("emit") {
        if let Some(path) = emit_path {
            std::fs::write(&path, warm.as_bytes()).expect("emit");
            println!("EMIT {path}");
        }
        return;
    }

    let t = Instant::now();
    let mut sink = 0usize;
    for _ in 0..iters {
        let out = write_csv_string(&frame).expect("csv");
        sink = sink.wrapping_add(out.len());
    }
    let d = t.elapsed();
    std::hint::black_box(sink);
    println!(
        "TIMING rows={rows} cols={cols} iters={iters} per_iter={:.3}ms",
        d.as_secs_f64() * 1e3 / iters as f64
    );
}
