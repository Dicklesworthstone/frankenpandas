//! Bench + golden for Column::floor/ceil/trunc — typed Float64/Int64 fast path.
//!
//! Run: cargo run -p fp-columnar --example bench_floor_ceil --release
//!
//! These scalar-looped self.values; an all-valid Float64/Int64 column now maps
//! the op over its contiguous buffer and re-ingests typed (mirror of abs/round).
//! Output is bit-identical.

use fp_columnar::Column;
use fp_types::{DType, NullKind, Scalar};
use std::time::Instant;

fn golden() -> String {
    let mut out = String::new();

    // All-valid Float64 (typed path): negatives, .5 ties, -0.0, large.
    let f = Column::from_f64_values(vec![1.2, 1.8, -1.2, -1.8, 2.5, -2.5, -0.0, 0.0]);
    out.push_str(&format!("floor_f={:?}\n", f.floor().unwrap().values()));
    out.push_str(&format!("ceil_f={:?}\n", f.ceil().unwrap().values()));
    out.push_str(&format!("trunc_f={:?}\n", f.trunc().unwrap().values()));

    // All-valid Int64 (typed path): becomes Float64, value-preserving.
    let i = Column::new(
        DType::Int64,
        vec![Scalar::Int64(3), Scalar::Int64(-4), Scalar::Int64(0)],
    )
    .unwrap();
    out.push_str(&format!("floor_i={:?}\n", i.floor().unwrap().values()));
    out.push_str(&format!("ceil_i={:?}\n", i.ceil().unwrap().values()));

    // Nullable Float64 (scalar path: missing -> NaN).
    let nf = Column::new(
        DType::Float64,
        vec![
            Scalar::Float64(1.7),
            Scalar::Null(NullKind::NaN),
            Scalar::Float64(-1.7),
        ],
    )
    .unwrap();
    out.push_str(&format!("floor_nf={:?}\n", nf.floor().unwrap().values()));
    out
}

fn main() {
    let g = golden();
    print!("GOLDEN_BEGIN\n{g}GOLDEN_END\n");

    let n: usize = 2_000_000;
    let data: Vec<f64> = (0..n).map(|i| (i as f64) * 0.0012345 - 1000.0).collect();
    let col = Column::from_f64_values(data);

    let _ = col.floor().unwrap(); // warmup

    let t = Instant::now();
    let r = col.floor().unwrap();
    let d = t.elapsed();
    assert_eq!(r.len(), n);

    println!("TIMING n={n} floor={:.3}ms", d.as_secs_f64() * 1e3);
}
