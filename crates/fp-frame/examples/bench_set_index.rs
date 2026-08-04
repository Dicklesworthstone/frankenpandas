//! Bench for DataFrame::set_index typed column→index label construction.
//!
//! Run:
//!   cargo run -p fp-frame --example bench_set_index --release -- 1000000 50 float64 drop

use std::{collections::BTreeMap, hint::black_box, time::Instant};

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::{Index, IndexLabel};
use fp_types::{DType, Scalar};

#[derive(Clone, Copy)]
enum Mode {
    Int64Scalar,
    Int64Typed,
    Float64,
    Utf8,
    Datetime64,
    Bool,
    Timedelta64,
}

impl Mode {
    fn parse(raw: &str) -> Self {
        match raw {
            "int64_scalar" => Self::Int64Scalar,
            "int64" | "int64_typed" => Self::Int64Typed,
            "float64" => Self::Float64,
            "utf8" => Self::Utf8,
            "datetime64" => Self::Datetime64,
            "bool" => Self::Bool,
            "timedelta64" => Self::Timedelta64,
            other => panic!(
                "unknown set_index mode {other:?}; expected int64_scalar, int64_typed, float64, utf8, datetime64, bool, or timedelta64"
            ),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Int64Scalar => "int64_scalar",
            Self::Int64Typed => "int64_typed",
            Self::Float64 => "float64",
            Self::Utf8 => "utf8",
            Self::Datetime64 => "datetime64",
            Self::Bool => "bool",
            Self::Timedelta64 => "timedelta64",
        }
    }
}

fn utf8_key_column(n: usize) -> Column {
    let mut bytes = Vec::with_capacity(n * 9);
    let mut offsets = Vec::with_capacity(n + 1);
    offsets.push(0);
    for i in 0..n {
        let label = format!("k{i:08}");
        bytes.extend_from_slice(label.as_bytes());
        offsets.push(bytes.len());
    }
    Column::from_utf8_contiguous(bytes, offsets)
}

fn key_column(n: usize, mode: Mode) -> Column {
    match mode {
        Mode::Int64Scalar => {
            Column::from_values((0..n as i64).map(|x| Scalar::Int64(x * 3)).collect()).unwrap()
        }
        Mode::Int64Typed => Column::from_i64_values((0..n as i64).map(|x| x * 3).collect()),
        Mode::Float64 => Column::from_f64_values((0..n).map(|i| i as f64 + 0.25).collect()),
        Mode::Utf8 => utf8_key_column(n),
        Mode::Datetime64 => Column::from_datetime64_values(
            (0..n as i64)
                .map(|i| 1_577_836_800_000_000_000_i64 + i * 1_000_000_000)
                .collect(),
        ),
        Mode::Bool => Column::from_bool_values((0..n).map(|i| i % 2 == 0).collect()),
        Mode::Timedelta64 => Column::new(
            DType::Timedelta64,
            (0..n as i64)
                .map(|i| Scalar::Timedelta64(i * 1_000_000_000))
                .collect(),
        )
        .unwrap(),
    }
}

fn build(n: usize, mode: Mode) -> DataFrame {
    let index = Index::new((0..n as i64).map(IndexLabel::Int64).collect());
    let mut cols = BTreeMap::new();
    cols.insert("a".to_string(), key_column(n, mode));
    cols.insert(
        "b".to_string(),
        Column::from_i64_values((0..n as i64).collect()),
    );
    DataFrame::new_with_column_order(index, cols, vec!["a".to_string(), "b".to_string()]).unwrap()
}

fn parse_drop(arg: Option<&String>) -> bool {
    match arg.map(String::as_str) {
        Some("drop" | "true" | "1") => true,
        Some("keep" | "false" | "0") | None => false,
        Some(other) => panic!("unknown drop mode {other:?}; expected drop/true/1 or keep/false/0"),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let n: usize = args
        .get(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(1_000_000);
    let iters: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(50);
    let mode = args.get(3).map_or(Mode::Int64Scalar, |s| Mode::parse(s));
    let drop = parse_drop(args.get(4));
    let df = build(n, mode);
    let mut best = u128::MAX;
    for _ in 0..iters {
        let t = Instant::now();
        let out = df.set_index("a", drop).expect("set_index");
        let e = t.elapsed().as_nanos();
        black_box(&out);
        if e < best {
            best = e;
        }
    }
    println!(
        "set_index mode={} drop={drop} n={n} iters={iters}: best={best}ns",
        mode.as_str()
    );
}
