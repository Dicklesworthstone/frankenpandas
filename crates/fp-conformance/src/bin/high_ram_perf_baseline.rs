#![forbid(unsafe_code)]

use std::{
    fs,
    hint::black_box,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use fp_frame::{DataFrame, Series};
use fp_index::IndexLabel;
use fp_io::{read_csv_str, write_csv_string};
use fp_join::{JoinType, MergedDataFrame, merge_dataframes};
use fp_runtime::{EvidenceLedger, RuntimePolicy};
use fp_types::Scalar;
use serde::Serialize;

type AnyError = Box<dyn std::error::Error>;

#[derive(Debug, Serialize)]
struct BenchmarkReport {
    schema_version: u32,
    bead: String,
    profile_name: String,
    generated_at_unix_ms: u128,
    command: String,
    config: RunConfig,
    host: HostFingerprint,
    workloads: Vec<WorkloadMeasurement>,
    delta_artifact_contract: DeltaArtifactContract,
    notes: Vec<String>,
}

#[derive(Debug, Serialize)]
struct RunConfig {
    rows: usize,
    iters: usize,
    warmup: usize,
    frame_cols: usize,
    key_cardinality: usize,
    join_type: String,
    alignment_overlap_rows: usize,
}

#[derive(Debug, Serialize)]
struct HostFingerprint {
    os: String,
    arch: String,
    available_parallelism: usize,
    rss_high_water_source: String,
}

#[derive(Debug, Serialize)]
struct WorkloadMeasurement {
    name: String,
    category: String,
    rows_in: usize,
    rows_out: usize,
    iters: usize,
    mean_ms: f64,
    p50_ms: f64,
    p95_ms: f64,
    p99_ms: f64,
    throughput_rows_per_sec: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    throughput_bytes_per_sec: Option<f64>,
    input_bytes_estimate: usize,
    output_bytes_estimate: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    io_payload_bytes: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rss_high_water_kib: Option<u64>,
    checksum: f64,
}

#[derive(Debug, Serialize)]
struct DeltaArtifactContract {
    required_fields: Vec<String>,
    compare_key: String,
    regression_rule: String,
    pandas_companion: String,
}

#[derive(Debug, Clone, Default)]
struct WorkloadOutput {
    rows_out: usize,
    output_bytes_estimate: usize,
    io_payload_bytes: Option<usize>,
    checksum: f64,
}

fn parse_arg<T: std::str::FromStr>(name: &str, default: T) -> T {
    let flag = format!("--{name}");
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == flag
            && let Some(value) = args.next()
            && let Ok(parsed) = value.parse::<T>()
        {
            return parsed;
        }
    }
    default
}

fn parse_string_arg(name: &str, default: &str) -> String {
    let flag = format!("--{name}");
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == flag
            && let Some(value) = args.next()
        {
            return value;
        }
    }
    default.to_owned()
}

fn quantile_ns(sorted_ns: &[u128], q: f64) -> u128 {
    if sorted_ns.is_empty() {
        return 0;
    }
    let max_idx = sorted_ns.len() - 1;
    let idx = ((max_idx as f64) * q).round().clamp(0.0, max_idx as f64) as usize;
    sorted_ns
        .get(idx)
        .copied()
        .unwrap_or_else(|| sorted_ns.last().copied().unwrap_or_default())
}

fn now_unix_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_millis()
}

fn rss_high_water_kib() -> Option<u64> {
    let status = fs::read_to_string("/proc/self/status").ok()?;
    status.lines().find_map(|line| {
        let rest = line.strip_prefix("VmHWM:")?;
        rest.split_whitespace().next()?.parse::<u64>().ok()
    })
}

fn dataframe_rows(frame: &DataFrame) -> usize {
    frame.len()
}

fn dataframe_memory_bytes(frame: &DataFrame) -> usize {
    frame
        .memory_usage()
        .ok()
        .map(|usage| {
            usage
                .values()
                .iter()
                .filter_map(|value| match value {
                    Scalar::Int64(v) if *v >= 0 => Some(*v as usize),
                    _ => None,
                })
                .sum()
        })
        .unwrap_or_default()
}

fn checksum_series(series: &Series) -> f64 {
    series
        .values()
        .iter()
        .filter_map(|value| value.to_f64().ok())
        .sum()
}

fn checksum_frame(frame: &DataFrame) -> f64 {
    frame
        .columns()
        .values()
        .flat_map(|column| column.values())
        .filter_map(|value| value.to_f64().ok())
        .sum()
}

fn merged_rows(frame: &MergedDataFrame) -> usize {
    frame.index.len()
}

fn merged_memory_bytes(frame: &MergedDataFrame) -> usize {
    let index_bytes = frame.index.len() * std::mem::size_of::<IndexLabel>();
    let column_bytes = frame
        .columns
        .values()
        .map(|column| column.len() * std::mem::size_of::<Scalar>())
        .sum::<usize>();
    index_bytes + column_bytes
}

fn checksum_merged(frame: &MergedDataFrame) -> f64 {
    frame
        .columns
        .values()
        .flat_map(|column| column.values())
        .filter_map(|value| value.to_f64().ok())
        .sum()
}

fn build_numeric_frame(prefix: &str, rows: usize, cols: usize) -> Result<DataFrame, AnyError> {
    let names = (0..cols)
        .map(|col| format!("{prefix}_{col}"))
        .collect::<Vec<_>>();
    let order = names.iter().map(String::as_str).collect::<Vec<&str>>();
    let data = names
        .iter()
        .enumerate()
        .map(|(col, name)| {
            let values = (0..rows)
                .map(|row| Scalar::Float64((row as f64 * 1.25) + col as f64))
                .collect::<Vec<_>>();
            (name.as_str(), values)
        })
        .collect::<Vec<_>>();
    Ok(DataFrame::from_dict(&order, data)?)
}

fn build_groupby_frame(rows: usize, key_cardinality: usize) -> Result<DataFrame, AnyError> {
    let cardinality = key_cardinality.max(1);
    let keys = (0..rows)
        .map(|row| Scalar::Int64((row % cardinality) as i64))
        .collect::<Vec<_>>();
    let values = (0..rows)
        .map(|row| Scalar::Float64((row as f64 * 0.5) + 1.0))
        .collect::<Vec<_>>();
    Ok(DataFrame::from_dict(
        &["key", "value"],
        vec![("key", keys), ("value", values)],
    )?)
}

fn build_join_frame(
    value_name: &str,
    rows: usize,
    key_cardinality: usize,
    multiplier: i64,
) -> Result<DataFrame, AnyError> {
    let cardinality = key_cardinality.max(1);
    let keys = (0..rows)
        .map(|row| Scalar::Int64((row % cardinality) as i64))
        .collect::<Vec<_>>();
    let values = (0..rows)
        .map(|row| Scalar::Int64(((row as i64 * multiplier + 11) % 10_007).abs()))
        .collect::<Vec<_>>();
    Ok(DataFrame::from_dict(
        &["id", value_name],
        vec![("id", keys), (value_name, values)],
    )?)
}

fn build_numeric_series(name: &str, rows: usize, offset: usize) -> Result<Series, AnyError> {
    let labels = (offset..offset + rows)
        .map(|row| IndexLabel::Int64(row as i64))
        .collect::<Vec<_>>();
    let values = (0..rows)
        .map(|row| Scalar::Float64((row as f64 * 1.5) + 0.25))
        .collect::<Vec<_>>();
    Ok(Series::from_values(name.to_owned(), labels, values)?)
}

fn measure_workload<F>(
    name: &str,
    category: &str,
    rows_in: usize,
    input_bytes_estimate: usize,
    iters: usize,
    warmup: usize,
    mut f: F,
) -> Result<WorkloadMeasurement, AnyError>
where
    F: FnMut() -> Result<WorkloadOutput, AnyError>,
{
    for _ in 0..warmup {
        let _ = black_box(f()?);
    }

    let mut durations = Vec::with_capacity(iters);
    let mut last_output = WorkloadOutput::default();
    for _ in 0..iters {
        let start = Instant::now();
        last_output = black_box(f()?);
        durations.push(start.elapsed().as_nanos());
    }
    durations.sort_unstable();

    let total_ns = durations.iter().sum::<u128>();
    let mean_ms = total_ns as f64 / iters as f64 / 1_000_000.0;
    let p50_ms = quantile_ns(&durations, 0.50) as f64 / 1_000_000.0;
    let p95_ms = quantile_ns(&durations, 0.95) as f64 / 1_000_000.0;
    let p99_ms = quantile_ns(&durations, 0.99) as f64 / 1_000_000.0;
    let mean_secs = mean_ms / 1_000.0;
    let throughput_rows_per_sec = if mean_secs > 0.0 {
        rows_in as f64 / mean_secs
    } else {
        0.0
    };
    let throughput_bytes_per_sec = last_output.io_payload_bytes.map(|bytes| {
        if mean_secs > 0.0 {
            bytes as f64 / mean_secs
        } else {
            0.0
        }
    });

    Ok(WorkloadMeasurement {
        name: name.to_owned(),
        category: category.to_owned(),
        rows_in,
        rows_out: last_output.rows_out,
        iters,
        mean_ms,
        p50_ms,
        p95_ms,
        p99_ms,
        throughput_rows_per_sec,
        throughput_bytes_per_sec,
        input_bytes_estimate,
        output_bytes_estimate: last_output.output_bytes_estimate,
        io_payload_bytes: last_output.io_payload_bytes,
        rss_high_water_kib: rss_high_water_kib(),
        checksum: last_output.checksum,
    })
}

fn run_filter(
    rows: usize,
    cols: usize,
    iters: usize,
    warmup: usize,
) -> Result<WorkloadMeasurement, AnyError> {
    let frame = build_numeric_frame("filter_col", rows, cols)?;
    let mask_labels = (0..rows)
        .map(|row| IndexLabel::Int64(row as i64))
        .collect::<Vec<_>>();
    let mask_values = (0..rows)
        .map(|row| Scalar::Bool(row % 2 == 0))
        .collect::<Vec<_>>();
    let mask = Series::from_values("mask".to_owned(), mask_labels, mask_values)?;
    let input_bytes = dataframe_memory_bytes(&frame) + mask.memory_usage();

    measure_workload(
        "filter_boolean_mask",
        "filter",
        rows,
        input_bytes,
        iters,
        warmup,
        || {
            let out = frame.filter_rows(&mask)?;
            Ok(WorkloadOutput {
                rows_out: dataframe_rows(&out),
                output_bytes_estimate: dataframe_memory_bytes(&out),
                io_payload_bytes: None,
                checksum: checksum_frame(&out),
            })
        },
    )
}

fn run_groupby(
    rows: usize,
    key_cardinality: usize,
    iters: usize,
    warmup: usize,
) -> Result<WorkloadMeasurement, AnyError> {
    let frame = build_groupby_frame(rows, key_cardinality)?;
    let input_bytes = dataframe_memory_bytes(&frame);

    measure_workload(
        "dataframe_groupby_sum",
        "groupby",
        rows,
        input_bytes,
        iters,
        warmup,
        || {
            let out = frame.groupby(&["key"])?.sum()?;
            Ok(WorkloadOutput {
                rows_out: dataframe_rows(&out),
                output_bytes_estimate: dataframe_memory_bytes(&out),
                io_payload_bytes: None,
                checksum: checksum_frame(&out),
            })
        },
    )
}

fn run_join(
    rows: usize,
    key_cardinality: usize,
    iters: usize,
    warmup: usize,
) -> Result<WorkloadMeasurement, AnyError> {
    let left = build_join_frame("left_value", rows, key_cardinality, 7)?;
    let right = build_join_frame("right_value", rows, key_cardinality, 13)?;
    let input_bytes = dataframe_memory_bytes(&left) + dataframe_memory_bytes(&right);

    measure_workload(
        "dataframe_inner_join",
        "join",
        rows * 2,
        input_bytes,
        iters,
        warmup,
        || {
            let out = merge_dataframes(&left, &right, "id", JoinType::Inner)?;
            Ok(WorkloadOutput {
                rows_out: merged_rows(&out),
                output_bytes_estimate: merged_memory_bytes(&out),
                io_payload_bytes: None,
                checksum: checksum_merged(&out),
            })
        },
    )
}

fn run_alignment(
    rows: usize,
    iters: usize,
    warmup: usize,
) -> Result<WorkloadMeasurement, AnyError> {
    let overlap = rows / 2;
    let left = build_numeric_series("left", rows, 0)?;
    let right = build_numeric_series("right", rows, rows - overlap)?;
    let input_bytes = left.memory_usage() + right.memory_usage();
    let policy = RuntimePolicy::hardened(Some(rows * 4));

    measure_workload(
        "series_add_outer_alignment",
        "alignment",
        rows * 2,
        input_bytes,
        iters,
        warmup,
        || {
            let mut ledger = EvidenceLedger::new();
            let out = left.add_with_policy(&right, &policy, &mut ledger)?;
            Ok(WorkloadOutput {
                rows_out: out.len(),
                output_bytes_estimate: out.memory_usage(),
                io_payload_bytes: None,
                checksum: checksum_series(&out),
            })
        },
    )
}

fn run_csv_roundtrip(
    rows: usize,
    cols: usize,
    iters: usize,
    warmup: usize,
) -> Result<WorkloadMeasurement, AnyError> {
    let frame = build_numeric_frame("io_col", rows, cols)?;
    let input_bytes = dataframe_memory_bytes(&frame);

    measure_workload(
        "csv_write_read_roundtrip",
        "io",
        rows,
        input_bytes,
        iters,
        warmup,
        || {
            let csv = write_csv_string(&frame)?;
            let out = read_csv_str(&csv)?;
            Ok(WorkloadOutput {
                rows_out: dataframe_rows(&out),
                output_bytes_estimate: dataframe_memory_bytes(&out),
                io_payload_bytes: Some(csv.len()),
                checksum: checksum_frame(&out),
            })
        },
    )
}

fn main() -> Result<(), AnyError> {
    let rows = parse_arg("rows", 10_000usize);
    let iters = parse_arg("iters", 10usize);
    let warmup = parse_arg("warmup", 2usize);
    let frame_cols = parse_arg("frame-cols", 5usize);
    let key_cardinality = parse_arg("key-cardinality", 512usize);
    let profile_name = parse_string_arg("profile", "smoke");
    let command = std::env::args().collect::<Vec<_>>().join(" ");

    let workloads = vec![
        run_filter(rows, frame_cols, iters, warmup)?,
        run_groupby(rows, key_cardinality, iters, warmup)?,
        run_join(rows, key_cardinality, iters, warmup)?,
        run_alignment(rows, iters, warmup)?,
        run_csv_roundtrip(rows, frame_cols, iters, warmup)?,
    ];

    let report = BenchmarkReport {
        schema_version: 1,
        bead: "br-frankenpandas-tn6qb.4".to_owned(),
        profile_name,
        generated_at_unix_ms: now_unix_ms(),
        command,
        config: RunConfig {
            rows,
            iters,
            warmup,
            frame_cols,
            key_cardinality,
            join_type: "inner".to_owned(),
            alignment_overlap_rows: rows / 2,
        },
        host: HostFingerprint {
            os: std::env::consts::OS.to_owned(),
            arch: std::env::consts::ARCH.to_owned(),
            available_parallelism: std::thread::available_parallelism()
                .map(usize::from)
                .unwrap_or(1),
            rss_high_water_source: "/proc/self/status VmHWM".to_owned(),
        },
        workloads,
        delta_artifact_contract: DeltaArtifactContract {
            required_fields: vec![
                "name".to_owned(),
                "category".to_owned(),
                "rows_in".to_owned(),
                "p50_ms".to_owned(),
                "p95_ms".to_owned(),
                "p99_ms".to_owned(),
                "throughput_rows_per_sec".to_owned(),
                "input_bytes_estimate".to_owned(),
                "output_bytes_estimate".to_owned(),
                "rss_high_water_kib".to_owned(),
            ],
            compare_key: "workloads[].name".to_owned(),
            regression_rule: "future deltas should flag p95 drift >10% on the same profile/host and >20% as a blocker".to_owned(),
            pandas_companion: "record pandas oracle rows beside matching workloads when pandas is available; keep missing oracle runs explicit rather than inferred".to_owned(),
        },
        notes: vec![
            "measurement-only harness; no optimization levers are applied".to_owned(),
            "default smoke profile is intentionally small; use --profile high-ram --rows 100000 --iters 30 on a quiet 64-core host for durable baselines".to_owned(),
            "RSS is process high-water at workload completion, so later workloads can inherit previous peaks; use separate process runs for isolated RSS attribution".to_owned(),
        ],
    };

    serde_json::to_writer_pretty(std::io::stdout().lock(), &report)?;
    println!();
    Ok(())
}
