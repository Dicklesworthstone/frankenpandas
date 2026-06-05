//! Bench + golden for merge_asof(..., by=...) — per-group two-pointer lever.
//!
//! Run: cargo run -p fp-join --example bench_asof_by --release
//!
//! The `by`-grouped asof path rebuilt the group's right-value vector and
//! re-scanned it from scratch for EVERY left row (O(L·R) per group). Grouping
//! the left rows and driving the existing monotonic two-pointer once per group
//! makes it O(L+R). Output is bit-identical.

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
                Scalar::Float64(f) => s.push_str(&format!("{f},")),
                Scalar::Null(_) => s.push_str("N,"),
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

fn golden() -> String {
    let mut out = String::new();
    // Two groups (a,b) interleaved; global `t` non-decreasing.
    // left:  t=[1,2,3,4,5,6]  g=[a,b,a,b,a,b]
    // right: t=[1,2,3,5]      g=[a,a,b,b]  rv=[10,11,12,13]
    let left = frame(vec![
        ("t", (1..=6).map(Scalar::Int64).collect()),
        (
            "g",
            ["a", "b", "a", "b", "a", "b"]
                .iter()
                .map(|s| Scalar::Utf8((*s).into()))
                .collect(),
        ),
    ]);
    let right = frame(vec![
        (
            "t",
            [1, 2, 3, 5].iter().map(|&x| Scalar::Int64(x)).collect(),
        ),
        (
            "g",
            ["a", "a", "b", "b"]
                .iter()
                .map(|s| Scalar::Utf8((*s).into()))
                .collect(),
        ),
        (
            "rv",
            [10, 11, 12, 13].iter().map(|&x| Scalar::Int64(x)).collect(),
        ),
    ]);

    let b = left
        .merge_asof_with_options(&right, "t", "backward", opts("g"))
        .unwrap();
    out.push_str(&format!("backward:{}\n", dump(&b)));
    let f = left
        .merge_asof_with_options(&right, "t", "forward", opts("g"))
        .unwrap();
    out.push_str(&format!("forward:{}\n", dump(&f)));
    let nr = left
        .merge_asof_with_options(&right, "t", "nearest", opts("g"))
        .unwrap();
    out.push_str(&format!("nearest:{}\n", dump(&nr)));

    // no-exact + tolerance
    let mut o2 = opts("g");
    o2.allow_exact_matches = false;
    o2.tolerance = Some(1.0);
    let t = left
        .merge_asof_with_options(&right, "t", "backward", o2)
        .unwrap();
    out.push_str(&format!("noexact_tol1:{}\n", dump(&t)));
    out
}

fn main() {
    let g = golden();
    print!("GOLDEN_BEGIN\n{g}GOLDEN_END\n");

    // Few groups => large per-group => the O(L·R) baseline bites.
    let n: usize = 40_000;
    let groups = 4i64;
    let lt: Vec<Scalar> = (0..n as i64).map(Scalar::Int64).collect();
    let lg: Vec<Scalar> = (0..n as i64).map(|i| Scalar::Int64(i % groups)).collect();
    let rt: Vec<Scalar> = (0..n as i64).map(Scalar::Int64).collect();
    let rg: Vec<Scalar> = (0..n as i64).map(|i| Scalar::Int64(i % groups)).collect();
    let rv: Vec<Scalar> = (0..n as i64).map(|i| Scalar::Int64(i * 2)).collect();
    let left = frame(vec![("t", lt), ("g", lg)]);
    let right = frame(vec![("t", rt), ("g", rg), ("rv", rv)]);

    let _ = left
        .merge_asof_with_options(&right, "t", "backward", opts("g"))
        .unwrap();

    let t0 = Instant::now();
    let m = left
        .merge_asof_with_options(&right, "t", "backward", opts("g"))
        .unwrap();
    let d = t0.elapsed();
    assert_eq!(m.index.len(), n);

    println!(
        "TIMING n={n} groups={groups} asof_by={:.3}ms",
        d.as_secs_f64() * 1e3
    );
}
