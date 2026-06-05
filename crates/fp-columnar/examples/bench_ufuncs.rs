//! Bench + golden for the transcendental ufunc family (sqrt/exp/log/trig/...).
//!
//! Run: cargo run -p fp-columnar --example bench_ufuncs --release
//!
//! Each scalar-looped self.values; an all-valid Float64/Int64 column now maps
//! the op over its contiguous buffer via the shared typed_float_unary helper
//! and re-ingests typed (mirror of abs/round/floor). NaN results (sqrt(-1),
//! log(-1), asin(2), ...) are re-marked missing exactly as the scalar path did.

use fp_columnar::Column;
use fp_types::{DType, NullKind, Scalar};
use std::time::Instant;

fn fcol(v: Vec<f64>) -> Column {
    Column::from_f64_values(v)
}

fn dump(c: &Column) -> String {
    let mut s = String::new();
    for v in c.values() {
        match v {
            Scalar::Float64(f) if f.is_nan() => s.push_str("nan,"),
            Scalar::Float64(f) => s.push_str(&format!("{},", f.to_bits())), // exact bits
            Scalar::Null(_) => s.push_str("N,"),
            other => s.push_str(&format!("{other:?},")),
        }
    }
    s
}

fn golden() -> String {
    // Edge inputs: negatives (domain errors -> NaN), 0, -0.0, >1 (asin/acos),
    // <1 (acosh), large, fractional.
    let f = fcol(vec![
        0.5, 1.5, -1.0, -0.5, 0.0, -0.0, 2.0, 100.0, 0.25, 3.14159,
    ]);
    let i = Column::new(
        DType::Int64,
        vec![
            Scalar::Int64(0),
            Scalar::Int64(1),
            Scalar::Int64(-2),
            Scalar::Int64(7),
        ],
    )
    .unwrap();
    // Nullable (scalar path) to confirm missing handling untouched.
    let nf = Column::new(
        DType::Float64,
        vec![
            Scalar::Float64(0.5),
            Scalar::Null(NullKind::NaN),
            Scalar::Float64(2.0),
        ],
    )
    .unwrap();

    let ops: Vec<(&str, fn(&Column) -> Result<Column, fp_columnar::ColumnError>)> = vec![
        ("sqrt", Column::sqrt),
        ("exp", Column::exp),
        ("log", Column::log),
        ("log10", Column::log10),
        ("log2", Column::log2),
        ("sin", Column::sin),
        ("cos", Column::cos),
        ("tan", Column::tan),
        ("asin", Column::asin),
        ("acos", Column::acos),
        ("atan", Column::atan),
        ("sinh", Column::sinh),
        ("cosh", Column::cosh),
        ("tanh", Column::tanh),
        ("asinh", Column::asinh),
        ("acosh", Column::acosh),
        ("atanh", Column::atanh),
        ("expm1", Column::expm1),
        ("log1p", Column::log1p),
        ("cbrt", Column::cbrt),
        ("radians", Column::radians),
        ("degrees", Column::degrees),
        ("reciprocal", Column::reciprocal),
    ];

    let mut out = String::new();
    for (name, op) in &ops {
        out.push_str(&format!("{name}_f:{}\n", dump(&op(&f).unwrap())));
        out.push_str(&format!("{name}_i:{}\n", dump(&op(&i).unwrap())));
        out.push_str(&format!("{name}_nf:{}\n", dump(&op(&nf).unwrap())));
    }
    out
}

fn main() {
    let g = golden();
    print!("GOLDEN_BEGIN\n{g}GOLDEN_END\n");

    let n: usize = 2_000_000;
    let data: Vec<f64> = (0..n).map(|i| 0.5 + (i as f64) * 1e-6).collect();
    let col = fcol(data);

    let _ = col.log().unwrap(); // warmup

    let t = Instant::now();
    let r = col.log().unwrap();
    let d = t.elapsed();
    assert_eq!(r.len(), n);

    println!("TIMING n={n} log={:.3}ms", d.as_secs_f64() * 1e3);
}
