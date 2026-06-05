//! Bench + golden for Column::round — typed all-valid Float64 fast path.
//!
//! Run: cargo run -p fp-columnar --example bench_round --release
//!
//! The Float64 path materialized lazy Scalars, built a Vec<Scalar::Float64>,
//! and re-validated in Column::new. An all-valid Float64 column now rounds over
//! its contiguous buffer and re-ingests typed (mirror of `abs`). Bit-identical.

use fp_columnar::Column;
use fp_types::{DType, Scalar};
use std::time::Instant;

fn golden() -> String {
    let mut out = String::new();

    // All-valid Float64 (typed fast path), several decimals incl negative.
    let f = Column::from_f64_values(vec![1.2345, 2.5, -2.5, 0.125, 1234.5678, -0.0]);
    for d in [0i32, 2, 3, -1, -2] {
        let r = f.round(d).unwrap();
        out.push_str(&format!("f64_d{d}={:?}\n", r.values()));
    }

    // Nullable Float64 (scalar path: NaN/missing present) — same formula.
    let nf = Column::new(
        DType::Float64,
        vec![
            Scalar::Float64(1.2345),
            Scalar::Null(fp_types::NullKind::NaN),
            Scalar::Float64(2.5),
        ],
    )
    .unwrap();
    out.push_str(&format!("nf_d2={:?}\n", nf.round(2).unwrap().values()));

    // Int64 passthrough (decimals>=0) and negative-decimals rounding.
    let i = Column::new(
        DType::Int64,
        vec![Scalar::Int64(1234), Scalar::Int64(-1567), Scalar::Int64(45)],
    )
    .unwrap();
    out.push_str(&format!("i64_d0={:?}\n", i.round(0).unwrap().values()));
    out.push_str(&format!("i64_dm2={:?}\n", i.round(-2).unwrap().values()));
    out
}

fn main() {
    let g = golden();
    print!("GOLDEN_BEGIN\n{g}GOLDEN_END\n");

    let n: usize = 2_000_000;
    let data: Vec<f64> = (0..n).map(|i| (i as f64) * 0.0012345 - 1000.0).collect();
    let col = Column::from_f64_values(data);

    let _ = col.round(2).unwrap(); // warmup

    let t = Instant::now();
    let r = col.round(2).unwrap();
    let d = t.elapsed();
    assert_eq!(r.len(), n);

    println!("TIMING n={n} round={:.3}ms", d.as_secs_f64() * 1e3);
}
