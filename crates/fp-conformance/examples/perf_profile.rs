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
//!   scenario ∈ { drop_duplicates, sort_single, filter_bool, inner_join, series_add, series_add_same, series_add_align, csv_read, csv_read_options, csv_read_no_na_filter }

use std::{collections::BTreeMap, fmt::Write as _, time::Instant};

use fp_frame::{DataFrame, Series};
use fp_index::{DuplicateKeep, Index, IndexLabel};
use fp_io::{CsvReadOptions, read_csv_str, read_csv_with_options};
use fp_join::{JoinType, merge_dataframes};
use fp_runtime::{EvidenceLedger, RuntimePolicy};
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

/// Deterministic ~24-char Utf8 Series with ~10% rows containing "needle"
/// mid-string — the canonical str.contains workload for the SIMD string-scan
/// campaign (every row is a real heap String, exercising the AoS wall).
fn build_str_series(n: usize) -> Series {
    let labels: Vec<IndexLabel> = (0..n as i64).map(IndexLabel::Int64).collect();
    let values: Vec<Scalar> = (0..n)
        .map(|i| {
            let h = (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 33;
            if i % 10 == 3 {
                Scalar::Utf8(format!("prefix_{h:08x}_needle_{:04}", i % 7919))
            } else {
                Scalar::Utf8(format!("prefix_{h:08x}_filler_{:04}", i % 7919))
            }
        })
        .collect();
    Series::from_values("s", labels, values).expect("str series")
}

fn build_series_pair_same(n: usize) -> (Series, Series) {
    // Identical indexes => alignment is gap-free and all-valid, exercising the
    // typed-output fast path in Column::aligned_binary_f64 (the common
    // `df['a'] + df['b']` shape where both columns share the frame index).
    let labels: Vec<IndexLabel> = (0..n as i64).map(IndexLabel::Int64).collect();
    let left_vals: Vec<Scalar> = (0..n).map(|i| Scalar::Float64(i as f64)).collect();
    let right_vals: Vec<Scalar> = (0..n).map(|i| Scalar::Float64(i as f64 * 2.0)).collect();
    let left = Series::from_values("l", labels.clone(), left_vals).expect("left series");
    let right = Series::from_values("r", labels, right_vals).expect("right series");
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

/// Build a many-column, all-finite, non-collinear numeric frame for the
/// pairwise corr/cov kernel benchmark. Values come from a cheap deterministic
/// hash so columns are linearly independent (corr != 1 off-diagonal) and
/// contain no NaN (exercises the all-finite Gram-matrix fast path).
fn build_corr_frame(n: usize, cols: usize) -> DataFrame {
    let labels: Vec<IndexLabel> = (0..n).map(|i| IndexLabel::Int64(i as i64)).collect();
    let index = Index::new(labels);
    let mut columns = BTreeMap::new();
    let mut column_order = Vec::with_capacity(cols);
    for c in 0..cols {
        let col_name = format!("c{c}");
        let values: Vec<Scalar> = (0..n)
            .map(|i| {
                // Deterministic splitmix-style hash -> finite f64 in ~[-1, 1).
                let mut z = (i as u64)
                    .wrapping_mul(0x9e37_79b9_7f4a_7c15)
                    .wrapping_add((c as u64).wrapping_mul(0xbf58_476d_1ce4_e5b9));
                z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
                z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
                z ^= z >> 31;
                let unit = (z >> 11) as f64 / (1u64 << 53) as f64; // [0, 1)
                Scalar::Float64(unit.mul_add(2.0, -1.0))
            })
            .collect();
        let column = fp_columnar::Column::from_values(values).expect("column");
        columns.insert(col_name.clone(), column);
        column_order.push(col_name);
    }
    DataFrame::new_with_column_order(index, columns, column_order).expect("frame")
}

fn build_csv_string(n: usize, cols: usize) -> String {
    let mut csv = String::with_capacity(n * cols * 15);
    let mut float_buffer = ryu::Buffer::new();
    for c in 0..cols {
        if c > 0 {
            csv.push(',');
        }
        write!(&mut csv, "c{c}").expect("writing to a String cannot fail");
    }
    csv.push('\n');
    for i in 0..n {
        for c in 0..cols {
            if c > 0 {
                csv.push(',');
            }
            let value = (i * (c + 1)) as f64 * 0.1;
            if value.fract() == 0.0 {
                write!(&mut csv, "{}", value as i64).expect("writing to a String cannot fail");
            } else {
                csv.push_str(float_buffer.format(value));
            }
        }
        csv.push('\n');
    }
    csv
}

fn read_csv_no_na_filter(csv: &str) -> DataFrame {
    let options = CsvReadOptions {
        na_filter: false,
        ..CsvReadOptions::default()
    };
    read_csv_with_options(csv, &options).expect("csv read no NA filter")
}

fn read_csv_options_default(csv: &str) -> DataFrame {
    read_csv_with_options(csv, &CsvReadOptions::default()).expect("csv read options")
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
    build_join_frame_offset(value_name, n, key_cardinality, multiplier, 0)
}

/// `build_join_frame` with the key domain shifted by `key_offset`, so two
/// frames with the same cardinality and a half-cardinality offset share ~50%
/// of their keys — the partial-overlap shape left/right/outer joins need to
/// exercise their unmatched-row (null-introducing) paths.
fn build_join_frame_offset(
    value_name: &str,
    n: usize,
    key_cardinality: usize,
    multiplier: i64,
    key_offset: i64,
) -> DataFrame {
    let cardinality = key_cardinality.max(1);
    let keys: Vec<Scalar> = (0..n)
        .map(|row| Scalar::Int64((row % cardinality) as i64 + key_offset))
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
        "df_corr" => build_corr_frame(n, 64).corr().expect("corr"),
        "df_cov" => build_corr_frame(n, 64).cov().expect("cov"),
        "df_spearman" => build_corr_frame(n, 64)
            .corr_method("spearman")
            .expect("spearman"),
        "df_kendall" => build_corr_frame(n, 32)
            .corr_method("kendall")
            .expect("kendall"),
        "inner_join" => {
            let left = build_join_frame("left_value", n, 512, 7);
            let right = build_join_frame("right_value", n, 512, 13);
            let out = merge_dataframes(&left, &right, "id", JoinType::Inner).expect("join");
            DataFrame::new_with_column_order(out.index, out.columns, out.column_order)
                .expect("join golden frame")
        }
        "join_1to1" => {
            let left = build_join_frame("left_value", n, n, 7);
            let right = build_join_frame("right_value", n, n, 13);
            let out = merge_dataframes(&left, &right, "id", JoinType::Inner).expect("join");
            DataFrame::new_with_column_order(out.index, out.columns, out.column_order)
                .expect("join golden frame")
        }
        "left_join" => {
            let left = build_join_frame("left_value", n, 512, 7);
            let right = build_join_frame_offset("right_value", n, 512, 13, 256);
            let out = merge_dataframes(&left, &right, "id", JoinType::Left).expect("join");
            DataFrame::new_with_column_order(out.index, out.columns, out.column_order)
                .expect("join golden frame")
        }
        "outer_join" => {
            let left = build_join_frame("left_value", n, 512, 7);
            let right = build_join_frame_offset("right_value", n, 512, 13, 256);
            let out = merge_dataframes(&left, &right, "id", JoinType::Outer).expect("join");
            DataFrame::new_with_column_order(out.index, out.columns, out.column_order)
                .expect("join golden frame")
        }
        "right_join" => {
            let left = build_join_frame("left_value", n, 512, 7);
            let right = build_join_frame_offset("right_value", n, 512, 13, 256);
            let out = merge_dataframes(&left, &right, "id", JoinType::Right).expect("join");
            DataFrame::new_with_column_order(out.index, out.columns, out.column_order)
                .expect("join golden frame")
        }
        "str_contains" => {
            let s = build_str_series(n);
            let out = s.str().contains("needle").expect("str contains");
            return print!("{}", golden_dump_series(&out));
        }
        "str_len" => {
            let s = build_str_series(n);
            let out = s.str().len().expect("str len");
            return print!("{}", golden_dump_series(&out));
        }
        "str_lower" => {
            let s = build_str_series(n);
            let out = s.str().lower().expect("str lower");
            return print!("{}", golden_dump_series(&out));
        }
        "str_chain" => {
            // lower -> strip -> contains: a 3-op pipeline whose intermediate
            // columns can stay contiguous (zero Scalar materialization).
            let s = build_str_series(n);
            let lowered = s.str().lower().expect("lower");
            let stripped = lowered.str().strip().expect("strip");
            let out = stripped.str().contains("needle").expect("contains");
            return print!("{}", golden_dump_series(&out));
        }
        "str_starts_chain" => {
            // lower -> startswith: the second op anchors at row offsets.
            let s = build_str_series(n);
            let lowered = s.str().lower().expect("lower");
            let out = lowered.str().startswith("prefix_0").expect("startswith");
            return print!("{}", golden_dump_series(&out));
        }
        "series_add" => {
            let (left, right) = build_series_pair(n);
            let out = left.add(&right).expect("series add");
            return print!("{}", golden_dump_series(&out));
        }
        "series_add_same" => {
            let (left, right) = build_series_pair_same(n);
            let out = left.add(&right).expect("series add same");
            return print!("{}", golden_dump_series(&out));
        }
        "series_add_align" => {
            let (left, right) = build_series_pair(n);
            let policy = RuntimePolicy::strict();
            let mut ledger = EvidenceLedger::new();
            let out = match left.add_with_policy(&right, &policy, &mut ledger) {
                Ok(out) => out,
                Err(err) => {
                    eprintln!("series add align golden failed: {err}");
                    std::process::exit(1);
                }
            };
            return print!("{}", golden_dump_series(&out));
        }
        "csv_read" => {
            let csv = build_csv_string(n, 10);
            read_csv_str(&csv).expect("csv read")
        }
        "csv_read_options" => {
            let csv = build_csv_string(n, 10);
            read_csv_options_default(&csv)
        }
        "csv_read_no_na_filter" => {
            let csv = build_csv_string(n, 10);
            read_csv_no_na_filter(&csv)
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
        "df_corr" => {
            let frame = build_corr_frame(n, 64);
            for _ in 0..iters {
                let out = frame.corr().expect("corr");
                sink = sink.wrapping_add(out.len());
            }
        }
        "df_cov" => {
            let frame = build_corr_frame(n, 64);
            for _ in 0..iters {
                let out = frame.cov().expect("cov");
                sink = sink.wrapping_add(out.len());
            }
        }
        "df_spearman" => {
            let frame = build_corr_frame(n, 64);
            for _ in 0..iters {
                let out = frame.corr_method("spearman").expect("spearman");
                sink = sink.wrapping_add(out.len());
            }
        }
        "df_kendall" => {
            // kendall is O(M^2) per pair; keep n small in the bench invocation.
            let frame = build_corr_frame(n, 32);
            for _ in 0..iters {
                let out = frame.corr_method("kendall").expect("kendall");
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
        "left_join" => {
            // 50% key overlap (right keys shifted by half the cardinality):
            // half the left rows match (fanout output), half are unmatched
            // (null-introduced right values -> Float64 promotion path).
            let left = build_join_frame("left_value", n, 512, 7);
            let right = build_join_frame_offset("right_value", n, 512, 13, 256);
            for _ in 0..iters {
                let out = merge_dataframes(&left, &right, "id", JoinType::Left).expect("join");
                sink = sink.wrapping_add(out.index.len());
            }
        }
        "outer_join" => {
            // 50% key overlap: matched fanout rows plus unmatched rows from
            // BOTH sides (null-introduced on each side).
            let left = build_join_frame("left_value", n, 512, 7);
            let right = build_join_frame_offset("right_value", n, 512, 13, 256);
            for _ in 0..iters {
                let out = merge_dataframes(&left, &right, "id", JoinType::Outer).expect("join");
                sink = sink.wrapping_add(out.index.len());
            }
        }
        "right_join" => {
            // 50% key overlap: half the right rows match (fanout), half are
            // unmatched (null-introduced left values, dtype preserved).
            let left = build_join_frame("left_value", n, 512, 7);
            let right = build_join_frame_offset("right_value", n, 512, 13, 256);
            for _ in 0..iters {
                let out = merge_dataframes(&left, &right, "id", JoinType::Right).expect("join");
                sink = sink.wrapping_add(out.index.len());
            }
        }
        "str_contains" => {
            let s = build_str_series(n);
            for _ in 0..iters {
                let out = s.str().contains("needle").expect("str contains");
                sink = sink.wrapping_add(out.len());
            }
        }
        "str_len" => {
            let s = build_str_series(n);
            for _ in 0..iters {
                let out = s.str().len().expect("str len");
                sink = sink.wrapping_add(out.len());
            }
        }
        "str_lower" => {
            let s = build_str_series(n);
            for _ in 0..iters {
                let out = s.str().lower().expect("str lower");
                sink = sink.wrapping_add(out.len());
            }
        }
        "str_chain" => {
            let s = build_str_series(n);
            for _ in 0..iters {
                let lowered = s.str().lower().expect("lower");
                let stripped = lowered.str().strip().expect("strip");
                let out = stripped.str().contains("needle").expect("contains");
                sink = sink.wrapping_add(out.len());
            }
        }
        "str_starts_chain" => {
            let s = build_str_series(n);
            for _ in 0..iters {
                let lowered = s.str().lower().expect("lower");
                let out = lowered.str().startswith("prefix_0").expect("startswith");
                sink = sink.wrapping_add(out.len());
            }
        }
        "inner_join_read" => {
            // Same fanout join as inner_join, but every iteration also READS
            // every output column (as_i64_slice sum), forcing any lazy output
            // representation to fully materialize — the downstream-consumer
            // gate for lazy join outputs (br-frankenpandas-3ad4n).
            let left = build_join_frame("left_value", n, 512, 7);
            let right = build_join_frame("right_value", n, 512, 13);
            for _ in 0..iters {
                let out = merge_dataframes(&left, &right, "id", JoinType::Inner).expect("join");
                let mut acc: i64 = 0;
                for name in &out.column_order {
                    let column = out.columns.get(name).expect("output column must exist");
                    let slice = column.as_i64_slice().expect("dense join output is Int64");
                    acc = acc.wrapping_add(slice.iter().sum::<i64>());
                }
                sink = sink.wrapping_add(out.index.len()).wrapping_add(acc as usize);
            }
        }
        "join_1to1" => {
            // Unique keys on both sides (cardinality = n) -> 1:1 join, output n
            // rows. Output gather is O(n); per-row composite-key extraction
            // (2n allocations) is the dominant non-gather cost here.
            let left = build_join_frame("left_value", n, n, 7);
            let right = build_join_frame("right_value", n, n, 13);
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
        "series_add_same" => {
            let (left, right) = build_series_pair_same(n);
            for _ in 0..iters {
                let out = left.add(&right).expect("series add same");
                sink = sink.wrapping_add(out.len());
            }
        }
        "series_add_align" => {
            let (left, right) = build_series_pair(n);
            let policy = RuntimePolicy::strict();
            for _ in 0..iters {
                let mut ledger = EvidenceLedger::new();
                let out = match left.add_with_policy(&right, &policy, &mut ledger) {
                    Ok(out) => out,
                    Err(err) => {
                        eprintln!("series add align failed: {err}");
                        std::process::exit(1);
                    }
                };
                sink = sink.wrapping_add(out.len());
            }
        }
        "csv_read" => {
            // Matches bench_runner::build_csv_string + io/csv_read shape:
            // 10 dense numeric columns, default pandas-style CSV parsing.
            let csv = build_csv_string(n, 10);
            for _ in 0..iters {
                let out = read_csv_str(&csv).expect("csv read");
                sink = sink.wrapping_add(out.len());
            }
        }
        "csv_read_options" => {
            let csv = build_csv_string(n, 10);
            for _ in 0..iters {
                let out = read_csv_options_default(&csv);
                sink = sink.wrapping_add(out.len());
            }
        }
        "csv_read_no_na_filter" => {
            let csv = build_csv_string(n, 10);
            for _ in 0..iters {
                let out = read_csv_no_na_filter(&csv);
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
