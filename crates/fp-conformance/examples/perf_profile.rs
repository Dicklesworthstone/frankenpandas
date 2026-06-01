//! Profiling-only harness (measurement, not optimization).
//!
//! Drives a single hot DataFrame operation in a tight loop so a sampling
//! profiler (samply / perf / cargo-flamegraph) can attribute CPU cost to the
//! responsible functions. Data shapes are kept identical to the `vs_pandas`
//! criterion bench so the flamegraph corresponds to the recorded baseline.
//!
//! Build (profilable) and run:
//!   RUSTFLAGS="-C force-frame-pointers=yes" \
//!     cargo build -p fp-conformance --profile release-perf --example perf_profile
//!   samply record ./target/release-perf/examples/perf_profile drop_duplicates 100000 200
//!
//! Args: <scenario> <n_rows> <iterations>
//!   scenario ∈ { drop_duplicates, sort_single, filter_bool }

use std::collections::BTreeMap;
use std::time::Instant;

use fp_frame::DataFrame;
use fp_index::{DuplicateKeep, Index, IndexLabel};
use fp_types::Scalar;

fn build_groupby_frame(n: usize, num_groups: usize) -> DataFrame {
    let keys: Vec<Scalar> = (0..n)
        .map(|i| Scalar::Int64((i % num_groups) as i64))
        .collect();
    let values: Vec<Scalar> = (0..n).map(|i| Scalar::Float64(i as f64 * 0.1)).collect();
    let labels: Vec<IndexLabel> = (0..n).map(|i| IndexLabel::Int64(i as i64)).collect();
    let index = Index::new(labels);
    let key_column = fp_columnar::Column::from_values(keys).expect("key column");
    let value_column = fp_columnar::Column::from_values(values).expect("value column");
    let mut columns = BTreeMap::new();
    columns.insert("k".to_string(), key_column);
    columns.insert("v".to_string(), value_column);
    let column_order = vec!["k".to_string(), "v".to_string()];
    DataFrame::new_with_column_order(index, columns, column_order).expect("frame")
}

fn build_numeric_frame(n: usize, cols: usize) -> DataFrame {
    let labels: Vec<IndexLabel> = (0..n).map(|i| IndexLabel::Int64(i as i64)).collect();
    let index = Index::new(labels);
    let mut columns = BTreeMap::new();
    let mut column_order = Vec::with_capacity(cols);
    for c in 0..cols {
        let col_name = format!("c{c}");
        let values: Vec<Scalar> = (0..n)
            .map(|i| Scalar::Float64((i * (c + 1)) as f64 * 0.1))
            .collect();
        let column = fp_columnar::Column::from_values(values).expect("column");
        columns.insert(col_name.clone(), column);
        column_order.push(col_name);
    }
    DataFrame::new_with_column_order(index, columns, column_order).expect("frame")
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let scenario = args.get(1).map(String::as_str).unwrap_or("drop_duplicates");
    let n: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(100_000);
    let iters: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(200);

    eprintln!("perf_profile: scenario={scenario} n={n} iters={iters}");
    let start = Instant::now();
    let mut sink: usize = 0;

    match scenario {
        "drop_duplicates" => {
            let frame = build_groupby_frame(n, 100);
            for _ in 0..iters {
                let out = frame
                    .drop_duplicates(None, DuplicateKeep::First, false)
                    .expect("dedup");
                sink = sink.wrapping_add(out.len());
            }
        }
        "sort_single" => {
            let frame = build_numeric_frame(n, 4);
            for _ in 0..iters {
                let out = frame.sort_values("c0", true).expect("sort");
                sink = sink.wrapping_add(out.len());
            }
        }
        "filter_bool" => {
            let frame = build_numeric_frame(n, 10);
            let mask: Vec<bool> = (0..n).map(|i| i % 2 == 0).collect();
            for _ in 0..iters {
                let out = frame.iloc_bool(&mask).expect("filter");
                sink = sink.wrapping_add(out.len());
            }
        }
        other => {
            eprintln!("unknown scenario: {other}");
            std::process::exit(2);
        }
    }

    let elapsed = start.elapsed();
    let per_iter_ms = elapsed.as_secs_f64() * 1e3 / iters as f64;
    eprintln!(
        "perf_profile: done {iters} iters in {:.3}s ({per_iter_ms:.3} ms/iter), sink={sink}",
        elapsed.as_secs_f64()
    );
}
