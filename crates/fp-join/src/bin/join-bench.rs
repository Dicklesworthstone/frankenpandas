#![forbid(unsafe_code)]

use std::time::Instant;

use fp_frame::DataFrame;
use fp_join::{JoinType, merge_dataframes};
use fp_types::Scalar;

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

fn parse_join_type(default: JoinType) -> JoinType {
    let flag = "--join-type";
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == flag
            && let Some(value) = args.next()
        {
            return match value.as_str() {
                "inner" => JoinType::Inner,
                "left" => JoinType::Left,
                "right" => JoinType::Right,
                "outer" => JoinType::Outer,
                "cross" => JoinType::Cross,
                _ => default,
            };
        }
    }
    default
}

fn quantile_ns(sorted_ns: &[u128], q: f64) -> u128 {
    if sorted_ns.is_empty() {
        return 0;
    }
    let idx = ((sorted_ns.len() - 1) as f64 * q).round() as usize;
    sorted_ns[idx]
}

fn build_frame(
    value_name: &str,
    rows: usize,
    key_cardinality: usize,
    multiplier: i64,
) -> Result<DataFrame, Box<dyn std::error::Error>> {
    let cardinality = key_cardinality.max(1);
    let mut id_values = Vec::with_capacity(rows);
    let mut payload_values = Vec::with_capacity(rows);

    for i in 0..rows {
        id_values.push(Scalar::Int64((i % cardinality) as i64));
        payload_values.push(Scalar::Int64(((i as i64 * multiplier + 11) % 10_007).abs()));
    }

    let frame = DataFrame::from_dict(
        &["id", value_name],
        vec![("id", id_values), (value_name, payload_values)],
    )?;
    Ok(frame)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let rows = parse_arg("rows", 50_000usize);
    let right_rows = parse_arg("right-rows", rows);
    let key_cardinality = parse_arg("key-cardinality", 1_024usize);
    let iters = parse_arg("iters", 20usize);
    let warmup = parse_arg("warmup", 3usize);
    let join_type = parse_join_type(JoinType::Inner);

    let left = build_frame("left_value", rows, key_cardinality, 7)?;
    let right = build_frame("right_value", right_rows, key_cardinality, 13)?;

    for _ in 0..warmup {
        let _ = merge_dataframes(&left, &right, "id", join_type)?;
    }

    let mut durations_ns = Vec::with_capacity(iters);
    let mut checksum = 0.0f64;
    let mut output_rows = 0usize;

    for _ in 0..iters {
        let start = Instant::now();
        let merged = merge_dataframes(&left, &right, "id", join_type)?;
        durations_ns.push(start.elapsed().as_nanos());

        let id_column = merged
            .columns
            .get("id")
            .ok_or("merge output missing id column")?;
        output_rows = id_column.len();

        let left_values = merged
            .columns
            .get("left_value")
            .ok_or("merge output missing left_value column")?;
        let right_values = merged
            .columns
            .get("right_value")
            .ok_or("merge output missing right_value column")?;

        for value in left_values
            .values()
            .iter()
            .chain(right_values.values().iter())
        {
            if let Ok(v) = value.to_f64() {
                checksum += v;
            }
        }
    }

    durations_ns.sort_unstable();
    let total_ns: u128 = durations_ns.iter().sum();
    let mean_ms = (total_ns as f64) / (iters as f64) / 1_000_000.0;
    let p50_ms = quantile_ns(&durations_ns, 0.50) as f64 / 1_000_000.0;
    let p95_ms = quantile_ns(&durations_ns, 0.95) as f64 / 1_000_000.0;
    let p99_ms = quantile_ns(&durations_ns, 0.99) as f64 / 1_000_000.0;

    let join_name = match join_type {
        JoinType::Inner => "inner",
        JoinType::Left => "left",
        JoinType::Right => "right",
        JoinType::Outer => "outer",
        JoinType::Cross => "cross",
    };

    println!(
        "join_bench join_type={join_name} rows={rows} right_rows={right_rows} key_cardinality={key_cardinality} warmup={warmup} iters={iters} output_rows={output_rows} mean_ms={mean_ms:.3} p50_ms={p50_ms:.3} p95_ms={p95_ms:.3} p99_ms={p99_ms:.3} checksum={checksum:.3}"
    );

    Ok(())
}
