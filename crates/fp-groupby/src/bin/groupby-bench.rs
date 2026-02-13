#![forbid(unsafe_code)]

use std::time::Instant;

use fp_frame::Series;
use fp_groupby::{GroupByOptions, groupby_sum};
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

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let rows = parse_arg("rows", 500_000usize);
    let key_cardinality = parse_arg("key-cardinality", 512usize);
    let iters = parse_arg("iters", 25usize);

    let mut index = Vec::with_capacity(rows);
    let mut key_values = Vec::with_capacity(rows);
    let mut value_values = Vec::with_capacity(rows);

    for i in 0..rows {
        index.push(Scalar::Int64(i as i64));
        key_values.push(Scalar::Int64((i % key_cardinality) as i64));
        value_values.push(Scalar::Int64(((i * 7 + 3) % 97) as i64));
    }

    let index_labels = index
        .iter()
        .map(|v| match v {
            Scalar::Int64(i) => (*i).into(),
            _ => unreachable!("index is always int"),
        })
        .collect::<Vec<_>>();
    let keys = Series::from_values("keys", index_labels.clone(), key_values)?;
    let values = Series::from_values("values", index_labels, value_values)?;

    let mut checksum = 0.0f64;
    let mut total_ns = 0u128;
    for _ in 0..iters {
        let start = Instant::now();
        let out = groupby_sum(
            &keys,
            &values,
            GroupByOptions::default(),
            &RuntimePolicy::strict(),
            &mut EvidenceLedger::new(),
        )?;
        total_ns += start.elapsed().as_nanos();

        for value in out.values() {
            if let Ok(v) = value.to_f64() {
                checksum += v;
            }
        }
    }

    let mean_ms = (total_ns as f64) / (iters as f64) / 1_000_000.0;
    println!(
        "groupby_bench rows={rows} key_cardinality={key_cardinality} iters={iters} mean_ms={mean_ms:.3} checksum={checksum:.3}"
    );

    Ok(())
}
