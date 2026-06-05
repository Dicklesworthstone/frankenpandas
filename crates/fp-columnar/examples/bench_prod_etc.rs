//! Bench + golden for typed prod / nan_to_num / sinc.
//!
//! Run: cargo run -p fp-columnar --example bench_prod_etc --release
//!
//! prod (reduction) folded over Vec<Scalar>; nan_to_num/sinc scalar-looped.
//! An all-valid Float64/Int64 column now takes a typed contiguous-buffer path
//! (prod mirrors sum; nan_to_num replaces ±Inf; sinc via typed_float_unary).
//! Bit-identical.

use fp_columnar::Column;
use fp_types::{DType, NullKind, Scalar};
use std::time::Instant;

fn fcol(v: Vec<f64>) -> Column {
    Column::from_f64_values(v)
}
fn icol(v: Vec<i64>) -> Column {
    Column::new(DType::Int64, v.into_iter().map(Scalar::Int64).collect()).unwrap()
}

fn dumpcol(c: &Column) -> String {
    let mut s = format!("[{:?}]", c.dtype());
    for v in c.values() {
        match v {
            Scalar::Float64(f) if f.is_nan() => s.push_str("nan,"),
            Scalar::Float64(f) => s.push_str(&format!("f{},", f.to_bits())),
            Scalar::Int64(i) => s.push_str(&format!("i{i},")),
            Scalar::Null(_) => s.push_str("N,"),
            other => s.push_str(&format!("{other:?},")),
        }
    }
    s
}

fn dumps(v: &Scalar) -> String {
    match v {
        Scalar::Float64(f) if f.is_nan() => "nan".into(),
        Scalar::Float64(f) => format!("f{}", f.to_bits()),
        Scalar::Null(_) => "N".into(),
        other => format!("{other:?}"),
    }
}

fn golden() -> String {
    let mut out = String::new();

    // prod: all-valid f64 (incl -0.0, large -> overflow to Inf), Int64, empty,
    // nullable.
    out.push_str(&format!("prod_f:{}\n", dumps(&fcol(vec![1.5, 2.0, -3.0, 0.5]).prod())));
    out.push_str(&format!("prod_zero:{}\n", dumps(&fcol(vec![2.0, -0.0, 3.0]).prod())));
    out.push_str(&format!("prod_empty:{}\n", dumps(&fcol(vec![]).prod())));
    out.push_str(&format!("prod_big:{}\n", dumps(&fcol(vec![1e300, 1e300]).prod())));
    out.push_str(&format!("prod_i:{}\n", dumps(&icol(vec![2, 3, 4]).prod())));
    let nf = Column::new(
        DType::Float64,
        vec![Scalar::Float64(2.0), Scalar::Null(NullKind::NaN), Scalar::Float64(4.0)],
    )
    .unwrap();
    out.push_str(&format!("prod_nf:{}\n", dumps(&nf.prod())));

    // nan_to_num: all-valid f64 with ±Inf, Int64, nullable (scalar path).
    let inf = fcol(vec![1.0, f64::INFINITY, -2.0, f64::NEG_INFINITY, 0.5]);
    out.push_str(&format!("ntn_f:{}\n", dumpcol(&inf.nan_to_num().unwrap())));
    out.push_str(&format!("ntn_i:{}\n", dumpcol(&icol(vec![5, -7]).nan_to_num().unwrap())));
    out.push_str(&format!("ntn_nf:{}\n", dumpcol(&nf.nan_to_num().unwrap())));

    // sinc: 0/-0.0/values, Int64, nullable.
    let sc = fcol(vec![0.0, -0.0, 0.5, 1.0, -1.0, 2.5]);
    out.push_str(&format!("sinc_f:{}\n", dumpcol(&sc.sinc().unwrap())));
    out.push_str(&format!("sinc_i:{}\n", dumpcol(&icol(vec![0, 1, 2]).sinc().unwrap())));
    out.push_str(&format!("sinc_nf:{}\n", dumpcol(&nf.sinc().unwrap())));
    out
}

fn main() {
    let g = golden();
    print!("GOLDEN_BEGIN\n{g}GOLDEN_END\n");

    let n: usize = 4_000_000;
    let col = fcol((0..n).map(|i| 1.0 + (i as f64) * 1e-9).collect());

    let _ = col.prod(); // warmup

    let t = Instant::now();
    let mut sink = 0.0;
    for _ in 0..5 {
        if let Scalar::Float64(p) = col.prod() {
            sink += p;
        }
    }
    let d = t.elapsed();
    std::hint::black_box(sink);

    println!("TIMING n={n} prod_x5={:.3}ms", d.as_secs_f64() * 1e3);
}
