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
//!   scenario ∈ { drop_duplicates, sort_single, filter_bool, inner_join }

use std::{collections::BTreeMap, time::Instant};

use fp_frame::{DataFrame, Series};
use fp_index::{DuplicateKeep, Index, IndexLabel};
use fp_join::{JoinType, merge_dataframes};
use fp_types::Scalar;

/// Two Float64 Series whose Int64 indexes overlap but are shifted by one
/// (left: 0..n, right: 1..=n), so `a + b` exercises the AACE outer-union
/// alignment path with n-1 matched labels and two unmatched endpoints —
/// matches br-frankenpandas-b75cc's series_add outer-alignment scenario.
fn build_series_pair(n: usize) -> (Series, Series) {
    let left_labels: Vec<IndexLabel> = (0..n as i64).map(IndexLabel::Int64).collect();
    let right_labels: Vec<IndexLabel> = (1..=n as i64).map(IndexLabel::Int64).collect();
    let left_vals: Vec<Scalar> = (0..n).map(|i| Scalar::Float64(i as f64)).collect();
    let right_vals: Vec<Scalar> = (0..n).map(|i| Scalar::Float64(i as f64 * 2.0)).collect();
    let left = Series::from_values("l", left_labels, left_vals).expect("left series");
    let right = Series::from_values("r", right_labels, right_vals).expect("right series");
    (left, right)
}

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

/// Join workload frame: `id` key column at fixed cardinality + one value
/// column. Matches `high_ram_perf_baseline::build_join_frame` so the flamegraph
/// corresponds to the recorded `dataframe_inner_join` baseline (~36x vs pandas).
fn build_join_frame(
    value_name: &str,
    n: usize,
    key_cardinality: usize,
    multiplier: i64,
) -> DataFrame {
    let cardinality = key_cardinality.max(1);
    let keys: Vec<Scalar> = (0..n)
        .map(|row| Scalar::Int64((row % cardinality) as i64))
        .collect();
    let values: Vec<Scalar> = (0..n)
        .map(|row| Scalar::Int64(((row as i64 * multiplier + 11) % 10_007).abs()))
        .collect();
    DataFrame::from_dict(
        &["id", value_name],
        vec![("id", keys), (value_name, values)],
    )
    .expect("join frame")
}

/// Deterministic serialization of a frame's observable state (index labels +
/// per-column dtype and values in column order). Used for the isomorphism
/// golden-output sha256 proof; it must be stable across the optimization.
fn golden_dump(df: &DataFrame) -> String {
    let mut s = String::new();
    s.push_str(&format!("nrows={}\n", df.len()));
    for label in df.index().labels() {
        s.push_str(&format!("{label:?}|"));
    }
    s.push('\n');
    for name in df.column_names() {
        let col = df.columns().get(name).expect("column");
        s.push_str(&format!("col {name} dtype={:?}\n", col.dtype()));
        for v in col.values() {
            s.push_str(&format!("{v:?};"));
        }
        s.push('\n');
    }
    s
}

fn golden_dump_series(s: &Series) -> String {
    let mut out = String::new();
    out.push_str(&format!("len={} dtype={:?}\n", s.len(), s.column().dtype()));
    for label in s.index().labels() {
        out.push_str(&format!("{label:?}|"));
    }
    out.push('\n');
    for v in s.column().values() {
        out.push_str(&format!("{v:?};"));
    }
    out.push('\n');
    out
}

fn run_golden(scenario: &str, n: usize) {
    let out = match scenario {
        "drop_duplicates" => build_groupby_frame(n, 100)
            .drop_duplicates(None, DuplicateKeep::First, false)
            .expect("dedup"),
        "sort_single" => build_numeric_frame(n, 4)
            .sort_values("c0", true)
            .expect("sort"),
        "filter_bool" => {
            let frame = build_numeric_frame(n, 10);
            let mask: Vec<bool> = (0..n).map(|i| i % 2 == 0).collect();
            frame.iloc_bool(&mask).expect("filter")
        }
        "inner_join" => {
            let left = build_join_frame("left_value", n, 512, 7);
            let right = build_join_frame("right_value", n, 512, 13);
            let out = merge_dataframes(&left, &right, "id", JoinType::Inner).expect("join");
            DataFrame::new(out.index, out.columns).expect("join golden frame")
        }
        "series_add" => {
            let (left, right) = build_series_pair(n);
            let out = left.add(&right).expect("series add");
            return print!("{}", golden_dump_series(&out));
        }
        other => {
            eprintln!("unknown golden scenario: {other}");
            std::process::exit(2);
        }
    };
    print!("{}", golden_dump(&out));
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let scenario = args.get(1).map(String::as_str).unwrap_or("drop_duplicates");
    let n: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(100_000);
    let iters: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(200);

    // Golden mode: `perf_profile golden <scenario> <n>` prints a deterministic
    // dump of the operation's output for sha256 isomorphism proofs.
    if scenario == "golden" {
        let gscenario = args.get(2).map(String::as_str).unwrap_or("drop_duplicates");
        let gn: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(5_000);
        run_golden(gscenario, gn);
        return;
    }

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
        "inner_join" => {
            // cardinality 512 matches the high_ram baseline; output fans out to
            // ~n^2/cardinality rows, which is where the ~36x cost lives.
            let left = build_join_frame("left_value", n, 512, 7);
            let right = build_join_frame("right_value", n, 512, 13);
            for _ in 0..iters {
                let out = merge_dataframes(&left, &right, "id", JoinType::Inner).expect("join");
                sink = sink.wrapping_add(out.index.len());
            }
        }
        "series_add" => {
            let (left, right) = build_series_pair(n);
            for _ in 0..iters {
                let out = left.add(&right).expect("series add");
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
