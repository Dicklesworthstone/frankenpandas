#![forbid(unsafe_code)]

use std::time::Instant;

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::Index;
use fp_join::{
    JoinType, MergeExecutionOptions, MergeValidateMode, MergedDataFrame, merge_dataframes,
    merge_dataframes_on_with_options,
};
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

fn has_flag(name: &str) -> bool {
    let flag = format!("--{name}");
    std::env::args().skip(1).any(|arg| arg == flag)
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

fn build_ordered_unique_frame(
    value_name: &str,
    rows: usize,
    multiplier: i64,
    even_keys: bool,
) -> Result<DataFrame, Box<dyn std::error::Error>> {
    let mut id_values = Vec::with_capacity(rows);
    let mut payload_values = Vec::with_capacity(rows);

    for i in 0..rows {
        let key = if even_keys { i.saturating_mul(2) } else { i };
        id_values.push(Scalar::Int64(key as i64));
        payload_values.push(Scalar::Int64(((i as i64 * multiplier + 11) % 10_007).abs()));
    }

    let frame = DataFrame::from_dict(
        &["id", value_name],
        vec![("id", id_values), (value_name, payload_values)],
    )?;
    Ok(frame)
}

fn lower_hex_digit(nibble: usize) -> u8 {
    match nibble {
        0..=9 => b'0' + nibble as u8,
        10..=15 => b'a' + (nibble as u8 - 10),
        _ => b'?',
    }
}

fn push_id_lower_hex_8(bytes: &mut Vec<u8>, value: usize) {
    bytes.extend_from_slice(b"id_");
    let digits = ((usize::BITS - value.leading_zeros()).div_ceil(4)).max(1) as usize;
    for _ in digits..8 {
        bytes.push(b'0');
    }
    for shift in (0..digits).rev() {
        bytes.push(lower_hex_digit((value >> (shift * 4)) & 0x0f));
    }
}

fn build_ordered_utf8_frame(
    value_name: &str,
    rows: usize,
    key_start: usize,
    multiplier: u64,
) -> Result<DataFrame, Box<dyn std::error::Error>> {
    let index = Index::new_known_unique_int64_unit_range(0, rows);
    let mut bytes = Vec::with_capacity(rows * 11);
    let mut offsets = Vec::with_capacity(rows + 1);
    offsets.push(0);
    for row in 0..rows {
        push_id_lower_hex_8(&mut bytes, key_start + row);
        offsets.push(bytes.len());
    }
    let values = (0..rows)
        .map(|row| ((row as u64).wrapping_mul(multiplier) % 10_003) as f64 * 0.25)
        .collect::<Vec<_>>();

    let mut columns = std::collections::BTreeMap::new();
    columns.insert(
        "id".to_string(),
        Column::from_utf8_contiguous(bytes, offsets),
    );
    columns.insert(value_name.to_string(), Column::from_f64_values(values));
    let column_order = vec!["id".to_string(), value_name.to_string()];
    Ok(DataFrame::new_with_column_order(
        index,
        columns,
        column_order,
    )?)
}

fn build_wide_sparse_frame(
    value_name: &str,
    rows: usize,
    multiplier: i64,
    stride: i64,
    rotate: bool,
) -> Result<DataFrame, Box<dyn std::error::Error>> {
    let stride = stride.max(1);
    let mut id_values = Vec::with_capacity(rows);
    let mut payload_values = Vec::with_capacity(rows);

    for i in 0..rows {
        let logical_pos = if rotate && rows > 0 {
            (i + rows / 2) % rows
        } else {
            rows.saturating_sub(1).saturating_sub(i)
        };
        id_values.push(Scalar::Int64((logical_pos as i64) * stride));
        payload_values.push(Scalar::Int64(((i as i64 * multiplier + 11) % 10_007).abs()));
    }

    let frame = DataFrame::from_dict(
        &["id", value_name],
        vec![("id", id_values), (value_name, payload_values)],
    )?;
    Ok(frame)
}

fn merge_once(
    left: &DataFrame,
    right: &DataFrame,
    join_type: JoinType,
    force_generic: bool,
) -> Result<MergedDataFrame, Box<dyn std::error::Error>> {
    if force_generic {
        return Ok(merge_dataframes_on_with_options(
            left,
            right,
            &["id"],
            &["id"],
            join_type,
            MergeExecutionOptions {
                validate_mode: Some(MergeValidateMode::ManyToMany),
                ..MergeExecutionOptions::default()
            },
        )?);
    }

    Ok(merge_dataframes(left, right, "id", join_type)?)
}

fn golden_dump(frame: &MergedDataFrame) -> String {
    let mut out = String::new();
    out.push_str(&format!("nrows={}\n", frame.index.len()));
    for label in frame.index.labels() {
        out.push_str(&format!("{label:?}|"));
    }
    out.push('\n');
    for name in &frame.column_order {
        let col = frame.columns.get(name).expect("column listed in order");
        out.push_str(&format!("col {name} dtype={:?}\n", col.dtype()));
        for value in col.values() {
            out.push_str(&format!("{value:?};"));
        }
        out.push('\n');
    }
    out
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let rows = parse_arg("rows", 50_000usize);
    let right_rows = parse_arg("right-rows", rows);
    let key_cardinality = parse_arg("key-cardinality", 1_024usize);
    let iters = parse_arg("iters", 20usize);
    let warmup = parse_arg("warmup", 3usize);
    let stride = parse_arg("stride", 1_048_576i64);
    let join_type = parse_join_type(JoinType::Inner);
    let ordered_unique = has_flag("ordered-unique");
    let wide_sparse = has_flag("wide-sparse");
    let str_ordered = has_flag("str-ordered");
    let force_generic = has_flag("force-generic");
    let golden = has_flag("golden");

    let (left, right) = if wide_sparse {
        (
            build_wide_sparse_frame("left_value", rows, 7, stride, false)?,
            build_wide_sparse_frame("right_value", right_rows, 13, stride, true)?,
        )
    } else if ordered_unique {
        (
            build_ordered_unique_frame("left_value", rows, 7, false)?,
            build_ordered_unique_frame("right_value", right_rows, 13, true)?,
        )
    } else if str_ordered {
        (
            build_ordered_utf8_frame("left_value", rows, 0, 37)?,
            build_ordered_utf8_frame(
                "right_value",
                right_rows,
                rows.saturating_sub(rows / 10),
                53,
            )?,
        )
    } else {
        (
            build_frame("left_value", rows, key_cardinality, 7)?,
            build_frame("right_value", right_rows, key_cardinality, 13)?,
        )
    };

    if golden {
        let merged = merge_once(&left, &right, join_type, force_generic)?;
        print!("{}", golden_dump(&merged));
        return Ok(());
    }

    for _ in 0..warmup {
        let _ = merge_once(&left, &right, join_type, force_generic)?;
    }

    let mut durations_ns = Vec::with_capacity(iters);
    let mut checksum = 0.0f64;
    let mut output_rows = 0usize;

    for _ in 0..iters {
        let start = Instant::now();
        let merged = merge_once(&left, &right, join_type, force_generic)?;
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
        "join_bench join_type={join_name} rows={rows} right_rows={right_rows} key_cardinality={key_cardinality} ordered_unique={ordered_unique} str_ordered={str_ordered} wide_sparse={wide_sparse} stride={stride} force_generic={force_generic} warmup={warmup} iters={iters} output_rows={output_rows} mean_ms={mean_ms:.3} p50_ms={p50_ms:.3} p95_ms={p95_ms:.3} p99_ms={p99_ms:.3} checksum={checksum:.3}"
    );

    Ok(())
}
