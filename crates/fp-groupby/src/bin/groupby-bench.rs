#![forbid(unsafe_code)]

use std::time::Instant;

use fp_frame::Series;
use fp_groupby::{
    AggFunc, GroupByOptions, groupby_agg, groupby_count, groupby_first, groupby_last, groupby_max,
    groupby_mean, groupby_median, groupby_min, groupby_nunique, groupby_prod, groupby_size,
    groupby_std, groupby_sum, groupby_var,
};
use fp_runtime::{EvidenceLedger, RuntimePolicy};
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

fn has_flag(name: &str) -> bool {
    let flag = format!("--{name}");
    std::env::args().any(|a| a == flag)
}

fn run_agg(
    agg: &str,
    keys: &Series,
    values: &Series,
) -> Result<Series, Box<dyn std::error::Error>> {
    let opts = GroupByOptions::default();
    let policy = RuntimePolicy::strict();
    let mut ledger = EvidenceLedger::new();
    let out = match agg {
        "mean" => groupby_mean(keys, values, opts, &policy, &mut ledger)?,
        "count" => groupby_count(keys, values, opts, &policy, &mut ledger)?,
        "size" => groupby_size(keys, values, opts, &policy, &mut ledger)?,
        "first" => groupby_first(keys, values, opts, &policy, &mut ledger)?,
        "last" => groupby_last(keys, values, opts, &policy, &mut ledger)?,
        "min" => groupby_min(keys, values, opts, &policy, &mut ledger)?,
        "max" => groupby_max(keys, values, opts, &policy, &mut ledger)?,
        "prod" => groupby_prod(keys, values, opts, &policy, &mut ledger)?,
        "var" => groupby_var(keys, values, opts, &policy, &mut ledger)?,
        "std" => groupby_std(keys, values, opts, &policy, &mut ledger)?,
        "nunique" => groupby_nunique(keys, values, opts, &policy, &mut ledger)?,
        "median" => groupby_median(keys, values, opts, &policy, &mut ledger)?,
        "sum" => groupby_sum(keys, values, opts, &policy, &mut ledger)?,
        "agg-sum" => groupby_agg(keys, values, AggFunc::Sum, opts, &policy, &mut ledger)?,
        "agg-prod" => groupby_agg(keys, values, AggFunc::Prod, opts, &policy, &mut ledger)?,
        "agg-var" => groupby_agg(keys, values, AggFunc::Var, opts, &policy, &mut ledger)?,
        "agg-std" => groupby_agg(keys, values, AggFunc::Std, opts, &policy, &mut ledger)?,
        "agg-median" => groupby_agg(keys, values, AggFunc::Median, opts, &policy, &mut ledger)?,
        "agg-nunique" => groupby_agg(keys, values, AggFunc::Nunique, opts, &policy, &mut ledger)?,
        other => return Err(format!("unknown agg '{other}'").into()),
    };
    Ok(out)
}

/// Order-sensitive FNV-1a digest over the output (label, value) pairs.
fn digest(out: &Series) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut mix = |x: u64| {
        h ^= x;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    };
    for label in out.index().labels() {
        mix(format!("{label:?}")
            .bytes()
            .fold(0u64, |a, b| a.wrapping_mul(131).wrapping_add(u64::from(b))));
    }
    for v in out.values() {
        match v {
            Scalar::Int64(i) => mix(*i as u64),
            Scalar::Float64(f) => mix(f.to_bits()),
            Scalar::Null(_) => mix(0xDEAD_BEEF),
            other => mix(format!("{other:?}")
                .bytes()
                .fold(0u64, |a, b| a.wrapping_mul(131).wrapping_add(u64::from(b)))),
        }
    }
    h
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let rows = parse_arg("rows", 500_000usize);
    let key_cardinality = parse_arg("key-cardinality", 512usize);
    let iters = parse_arg("iters", 25usize);
    let agg = parse_arg("agg", "mean".to_string());
    let key_kind = parse_arg("key-kind", "int64".to_string());
    let value_kind = parse_arg("value-kind", "int64".to_string());
    let golden = has_flag("golden");

    let mut index_labels = Vec::with_capacity(rows);
    let mut key_values = Vec::with_capacity(rows);
    let mut value_values = Vec::with_capacity(rows);
    let utf8_keys = if key_kind == "utf8" {
        (0..key_cardinality)
            .map(|key| format!("key_{key:06}"))
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    for i in 0..rows {
        index_labels.push((i as i64).into());
        key_values.push(match key_kind.as_str() {
            "int64" => Scalar::Int64((i % key_cardinality) as i64),
            "utf8" => Scalar::Utf8(utf8_keys[i % key_cardinality].clone()),
            other => return Err(format!("unknown key-kind '{other}'").into()),
        });
        // Sprinkle nulls to exercise the skipna fold paths.
        if i % 37 == 0 {
            value_values.push(Scalar::Null(fp_types::NullKind::NaN));
        } else {
            value_values.push(match value_kind.as_str() {
                "int64" => Scalar::Int64(((i * 7 + 3) % 97) as i64),
                "float64" => Scalar::Float64(1.0 + (((i * 7 + 3) % 97) as f64 / 97.0)),
                other => return Err(format!("unknown value-kind '{other}'").into()),
            });
        }
    }
    let keys = Series::from_values("keys", index_labels.clone(), key_values)?;
    let values = Series::from_values("values", index_labels, value_values)?;

    if golden {
        let out = run_agg(&agg, &keys, &values)?;
        println!(
            "groupby_golden agg={agg} key_kind={key_kind} value_kind={value_kind} rows={rows} key_cardinality={key_cardinality} out_rows={} digest={:016x}",
            out.len(),
            digest(&out)
        );
        return Ok(());
    }

    let mut checksum = 0.0f64;
    let mut total_ns = 0u128;
    for _ in 0..iters {
        let start = Instant::now();
        let out = run_agg(&agg, &keys, &values)?;
        total_ns += start.elapsed().as_nanos();
        checksum += out.len() as f64;
    }
    let mean_ms = (total_ns as f64) / (iters as f64) / 1_000_000.0;
    println!(
        "groupby_bench agg={agg} key_kind={key_kind} value_kind={value_kind} rows={rows} key_cardinality={key_cardinality} iters={iters} mean_ms={mean_ms:.3} checksum={checksum:.3}"
    );
    Ok(())
}
