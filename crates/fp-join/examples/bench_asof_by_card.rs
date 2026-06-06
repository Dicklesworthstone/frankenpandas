//! Bench + golden for merge_asof(..., by=<high-cardinality key>).
//!
//! The grouped asof path builds each row's `by` key with `format!("{val:?}")`
//! and clones a `Vec<String>` per row into the grouping map. At high `by`
//! cardinality (the canonical "asof join by symbol") that per-cell string
//! formatting and allocation dominates the actual two-pointer sweep.
//!
//! Run: cargo run -p fp-join --example bench_asof_by_card --release -- [bench|golden]

use std::time::Instant;

use fp_frame::DataFrame;
use fp_join::{DataFrameMergeExt, MergeAsofOptions, MergedDataFrame};
use fp_types::Scalar;

fn frame(cols: Vec<(&str, Vec<Scalar>)>) -> DataFrame {
    let names: Vec<&str> = cols.iter().map(|(n, _)| *n).collect();
    DataFrame::from_dict(&names, cols).expect("frame")
}

fn dump(m: &MergedDataFrame) -> String {
    let mut s = String::new();
    for name in &m.column_order {
        let col = m.columns.get(name).unwrap();
        s.push_str(name);
        s.push('=');
        for v in col.values() {
            match v {
                Scalar::Int64(i) => s.push_str(&format!("{i},")),
                Scalar::Float64(f) if f.is_nan() => s.push_str("nan,"),
                Scalar::Float64(f) => s.push_str(&format!("{},", f.to_bits())),
                Scalar::Null(_) => s.push_str("N,"),
                Scalar::Utf8(u) => s.push_str(&format!("{u},")),
                other => s.push_str(&format!("{other:?},")),
            }
        }
        s.push('|');
    }
    s
}

fn opts(by: &str) -> MergeAsofOptions {
    MergeAsofOptions {
        allow_exact_matches: true,
        tolerance: None,
        by: Some(vec![by.to_string()]),
    }
}

/// Build a left/right pair: `n` rows each, `cardinality` distinct string `by`
/// keys ("sym{k}"), globally non-decreasing integer `t`, integer payload `rv`.
fn build(n: usize, cardinality: i64) -> (DataFrame, DataFrame) {
    let lt: Vec<Scalar> = (0..n as i64).map(Scalar::Int64).collect();
    let lg: Vec<Scalar> = (0..n as i64)
        .map(|i| Scalar::Utf8(format!("sym{}", i % cardinality)))
        .collect();
    let rt: Vec<Scalar> = (0..n as i64).map(Scalar::Int64).collect();
    let rg: Vec<Scalar> = (0..n as i64)
        .map(|i| Scalar::Utf8(format!("sym{}", i % cardinality)))
        .collect();
    let rv: Vec<Scalar> = (0..n as i64).map(|i| Scalar::Int64(i * 2)).collect();
    let left = frame(vec![("t", lt), ("g", lg)]);
    let right = frame(vec![("t", rt), ("g", rg), ("rv", rv)]);
    (left, right)
}

fn main() {
    let mode = std::env::args().nth(1).unwrap_or_else(|| "bench".to_string());

    if mode == "golden" {
        // Smaller deterministic case spanning backward/forward/nearest.
        let (left, right) = build(2000, 137);
        let mut out = String::new();
        for dir in ["backward", "forward", "nearest"] {
            let m = left
                .merge_asof_with_options(&right, "t", dir, opts("g"))
                .unwrap();
            out.push_str(&format!("{dir}:{}\n", dump(&m)));
        }
        print!("{out}");
        return;
    }

    let n: usize = 1_000_000;
    let cardinality = 20_000i64;
    let (left, right) = build(n, cardinality);

    // warmup
    let _ = left
        .merge_asof_with_options(&right, "t", "backward", opts("g"))
        .unwrap();

    let iters = 10;
    let mut best = f64::INFINITY;
    for _ in 0..iters {
        let t0 = Instant::now();
        let m = left
            .merge_asof_with_options(&right, "t", "backward", opts("g"))
            .unwrap();
        let d = t0.elapsed().as_secs_f64();
        std::hint::black_box(&m);
        if d < best {
            best = d;
        }
    }
    println!(
        "asof_by n={n} cardinality={cardinality} best={:.3}ms",
        best * 1e3
    );
}
