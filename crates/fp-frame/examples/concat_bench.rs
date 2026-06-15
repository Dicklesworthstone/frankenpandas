//! A/B bench + golden harness for default-RangeIndex construction sites
//! (br-frankenpandas-arr72): concat(ignore_index=true) and reset_index.
//!
//! Usage:
//!   concat_bench bench <rows_per_frame> <frames> <iters>   # per-iter ms
//!   concat_bench golden <rows_per_frame> <frames>          # deterministic dump
//!   concat_bench bench_reset <rows> <iters>
//!   concat_bench golden_reset <rows>

use std::{collections::BTreeMap, fmt::Write as _};

use fp_frame::{
    DataFrame, Series, concat_dataframes_with_ignore_index, concat_series_with_ignore_index,
};
use fp_index::{Index, IndexLabel};
use fp_types::Scalar;

// Pre-arr72 eager reimplementation, inlined here so A/B runs in ONE process on
// ONE worker (rch worker speed varies, so cross-invocation A/B is unreliable).
fn concat_series_eager(series_list: &[&Series], ignore_index: bool) -> Series {
    assert!(ignore_index);
    let total_len: usize = series_list.iter().map(|s| s.len()).sum();
    let mut values = Vec::with_capacity(total_len);
    for s in series_list {
        values.extend_from_slice(s.values());
    }
    let labels: Vec<IndexLabel> = (0..total_len as i64).map(IndexLabel::Int64).collect();
    Series::from_values("s", labels, values).expect("series")
}

fn build_series(rows: usize, seed: u64) -> Series {
    let labels: Vec<IndexLabel> = (0..rows)
        .map(|i| IndexLabel::Int64(i as i64 + seed as i64 * 10))
        .collect();
    let values: Vec<Scalar> = (0..rows)
        .map(|i| Scalar::Float64((i as f64).mul_add(0.25, seed as f64)))
        .collect();
    Series::from_values("s", labels, values).expect("series")
}

fn build_frame(rows: usize, cols: usize, seed: u64) -> DataFrame {
    let labels: Vec<IndexLabel> = (0..rows)
        .map(|i| IndexLabel::Int64(i as i64 + seed as i64 * 10))
        .collect();
    let mut columns = BTreeMap::new();
    let mut order = Vec::new();
    for c in 0..cols {
        let name = format!("c{c}");
        let column = fp_columnar::Column::from_f64_values(
            (0..rows)
                .map(|i| (i as f64).mul_add(0.25, (c as f64) * 1000.0 + seed as f64))
                .collect(),
        );
        columns.insert(name.clone(), column);
        order.push(name);
    }
    DataFrame::new_with_column_order(Index::new(labels), columns, order).expect("frame")
}

fn golden_dump(frame: &DataFrame) -> String {
    let mut out = String::new();
    let labels = frame.index().labels();
    writeln!(&mut out, "len={}", frame.len()).unwrap();
    // Full label sequence digestible: write every 997th label plus ends.
    for (i, label) in labels.iter().enumerate() {
        if i % 997 == 0 || i + 1 == labels.len() {
            writeln!(&mut out, "L{i}={label:?}").unwrap();
        }
    }
    for name in frame.column_names() {
        let col = frame.column(name).expect("col");
        let values = col.values();
        for (i, v) in values.iter().enumerate() {
            if i % 4999 == 0 || i + 1 == values.len() {
                if let Scalar::Float64(x) = v {
                    writeln!(&mut out, "{name}[{i}]={:016x}", x.to_bits()).unwrap();
                } else {
                    writeln!(&mut out, "{name}[{i}]={v:?}").unwrap();
                }
            }
        }
    }
    out
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mode = args.get(1).map(String::as_str).unwrap_or("bench");
    match mode {
        "bench" => {
            let rows: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(250_000);
            let frames_n: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(8);
            let iters: usize = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(20);
            let frames: Vec<DataFrame> = (0..frames_n)
                .map(|k| build_frame(rows, 4, k as u64))
                .collect();
            let refs: Vec<&DataFrame> = frames.iter().collect();
            let start = std::time::Instant::now();
            let mut sink = 0usize;
            for _ in 0..iters {
                let out = concat_dataframes_with_ignore_index(&refs, true).expect("concat");
                sink = sink.wrapping_add(out.len());
            }
            let elapsed = start.elapsed();
            eprintln!(
                "concat_bench: {iters} iters in {:.3}s ({:.3} ms/iter), sink={sink}",
                elapsed.as_secs_f64(),
                elapsed.as_secs_f64() * 1000.0 / iters as f64
            );
        }
        "golden" => {
            let rows: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(5_000);
            let frames_n: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(3);
            let frames: Vec<DataFrame> = (0..frames_n)
                .map(|k| build_frame(rows, 4, k as u64))
                .collect();
            let refs: Vec<&DataFrame> = frames.iter().collect();
            let out = concat_dataframes_with_ignore_index(&refs, true).expect("concat");
            print!("{}", golden_dump(&out));
        }
        "bench_reset" => {
            let rows: usize = args
                .get(2)
                .and_then(|s| s.parse().ok())
                .unwrap_or(2_000_000);
            let iters: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(20);
            let frame = build_frame(rows, 4, 7);
            let start = std::time::Instant::now();
            let mut sink = 0usize;
            for _ in 0..iters {
                let out = frame.reset_index(true).expect("reset_index");
                sink = sink.wrapping_add(out.len());
            }
            let elapsed = start.elapsed();
            eprintln!(
                "reset_bench: {iters} iters in {:.3}s ({:.3} ms/iter), sink={sink}",
                elapsed.as_secs_f64(),
                elapsed.as_secs_f64() * 1000.0 / iters as f64
            );
        }
        "golden_reset" => {
            let rows: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(5_000);
            let frame = build_frame(rows, 4, 7);
            let out = frame.reset_index(true).expect("reset_index");
            print!("{}", golden_dump(&out));
        }
        "bench_series" => {
            let rows: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(250_000);
            let series_n: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(8);
            let iters: usize = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(20);
            let series: Vec<Series> = (0..series_n)
                .map(|k| build_series(rows, k as u64))
                .collect();
            let refs: Vec<&Series> = series.iter().collect();
            let start = std::time::Instant::now();
            let mut sink = 0usize;
            for _ in 0..iters {
                let out = concat_series_with_ignore_index(&refs, true).expect("concat");
                sink = sink.wrapping_add(out.len());
            }
            let elapsed = start.elapsed();
            eprintln!(
                "concat_series_bench: {iters} iters in {:.3}s ({:.3} ms/iter), sink={sink}",
                elapsed.as_secs_f64(),
                elapsed.as_secs_f64() * 1000.0 / iters as f64
            );
        }
        "bench_series_ab" => {
            let rows: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(500_000);
            let series_n: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(8);
            let iters: usize = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(30);
            let series: Vec<Series> = (0..series_n)
                .map(|k| build_series(rows, k as u64))
                .collect();
            let refs: Vec<&Series> = series.iter().collect();
            // Interleave eager/lazy iterations to share any thermal/cache drift.
            let (mut eager_ns, mut lazy_ns, mut sink) = (0u128, 0u128, 0usize);
            for _ in 0..iters {
                let t = std::time::Instant::now();
                let oe = concat_series_eager(&refs, true);
                eager_ns += t.elapsed().as_nanos();
                sink = sink.wrapping_add(oe.len());
                let t = std::time::Instant::now();
                let ol = concat_series_with_ignore_index(&refs, true).expect("concat");
                lazy_ns += t.elapsed().as_nanos();
                sink = sink.wrapping_add(ol.len());
            }
            let em = eager_ns as f64 / 1e6 / iters as f64;
            let lm = lazy_ns as f64 / 1e6 / iters as f64;
            eprintln!(
                "concat_series_ab: eager={em:.3} ms/iter, lazy={lm:.3} ms/iter, ratio={:.3}x, sink={sink}",
                em / lm
            );
        }
        other => {
            eprintln!("unknown mode: {other}");
            std::process::exit(2);
        }
    }
}
