//! Bench + golden digest for Series::searchsorted_values (array form).
//!
//! Run: cargo run -p fp-frame --example bench_searchsorted --release
//!
//! searchsorted_values mapped each needle through `searchsorted`, which
//! re-materialized the whole column AND linear-scanned it = O(m·n) work plus
//! O(m·n) materialization. A sorted Series admits binary search: materialize
//! once, then partition_point per needle = O(n + m·log n). Bit-identical on
//! sorted input (the documented precondition); a sortedness guard falls back
//! to the exact linear scan for unsorted input so behavior is unchanged.

use std::time::Instant;

use fp_frame::Series;
use fp_index::IndexLabel;
use fp_types::Scalar;

fn s_i64(vals: Vec<i64>) -> Series {
    let idx: Vec<IndexLabel> = (0..vals.len() as i64).map(IndexLabel::Int64).collect();
    let sc: Vec<Scalar> = vals.into_iter().map(Scalar::Int64).collect();
    Series::from_values("s", idx, sc).unwrap()
}

fn s_scalars(vals: Vec<Scalar>) -> Series {
    let idx: Vec<IndexLabel> = (0..vals.len() as i64).map(IndexLabel::Int64).collect();
    Series::from_values("s", idx, vals).unwrap()
}

fn golden() -> String {
    let mut out = String::new();

    // Sorted Int64, both sides, exact + between + below + above.
    let s = s_i64(vec![10, 20, 20, 30, 40]);
    let needles: Vec<Scalar> = vec![5, 10, 15, 20, 35, 40, 50]
        .into_iter()
        .map(Scalar::Int64)
        .collect();
    out.push_str(&format!(
        "i64_left={:?}\n",
        s.searchsorted_values(&needles, "left").unwrap()
    ));
    out.push_str(&format!(
        "i64_right={:?}\n",
        s.searchsorted_values(&needles, "right").unwrap()
    ));

    // Sorted Float64 with trailing missing (NaN sorts last).
    let fs = s_scalars(vec![
        Scalar::Float64(1.5),
        Scalar::Float64(2.5),
        Scalar::Float64(2.5),
        Scalar::Float64(9.0),
        Scalar::Float64(f64::NAN),
    ]);
    let fn_needles = vec![
        Scalar::Float64(2.5),
        Scalar::Float64(0.0),
        Scalar::Float64(10.0),
        Scalar::Float64(f64::NAN), // missing needle
    ];
    out.push_str(&format!(
        "f64_left={:?}\n",
        fs.searchsorted_values(&fn_needles, "left").unwrap()
    ));
    out.push_str(&format!(
        "f64_right={:?}\n",
        fs.searchsorted_values(&fn_needles, "right").unwrap()
    ));

    // Sorted Utf8.
    let us = s_scalars(
        vec!["apple", "banana", "banana", "cherry"]
            .into_iter()
            .map(|x| Scalar::Utf8(x.to_string()))
            .collect(),
    );
    let u_needles: Vec<Scalar> = vec!["aardvark", "banana", "date"]
        .into_iter()
        .map(|x| Scalar::Utf8(x.to_string()))
        .collect();
    out.push_str(&format!(
        "utf8_left={:?}\n",
        us.searchsorted_values(&u_needles, "left").unwrap()
    ));
    out.push_str(&format!(
        "utf8_right={:?}\n",
        us.searchsorted_values(&u_needles, "right").unwrap()
    ));

    // UNSORTED input: must fall back to the exact linear-scan behavior.
    let uns = s_i64(vec![30, 10, 40, 20, 20]);
    let un_needles: Vec<Scalar> = vec![5, 20, 35, 40].into_iter().map(Scalar::Int64).collect();
    out.push_str(&format!(
        "unsorted_left={:?}\n",
        uns.searchsorted_values(&un_needles, "left").unwrap()
    ));
    out.push_str(&format!(
        "unsorted_right={:?}\n",
        uns.searchsorted_values(&un_needles, "right").unwrap()
    ));

    // Single-value searchsorted unchanged.
    out.push_str(&format!(
        "single_left={}\n",
        s.searchsorted(&Scalar::Int64(20), "left").unwrap()
    ));
    out.push_str(&format!(
        "single_right={}\n",
        s.searchsorted(&Scalar::Int64(20), "right").unwrap()
    ));
    out.push_str(&format!(
        "single_missing_left={}\n",
        fs.searchsorted(&Scalar::Float64(f64::NAN), "left").unwrap()
    ));
    out.push_str(&format!(
        "single_missing_right={}\n",
        fs.searchsorted(&Scalar::Float64(f64::NAN), "right")
            .unwrap()
    ));

    out
}

fn main() {
    let g = golden();
    print!("GOLDEN_BEGIN\n{g}GOLDEN_END\n");

    let n: usize = 40_000;
    let m: usize = 40_000;
    let s = s_i64((0..n as i64).map(|v| v * 2).collect());
    let needles: Vec<Scalar> = (0..m as i64).map(|v| Scalar::Int64(v * 2 + 1)).collect();

    // warmup
    let _ = s.searchsorted_values(&needles, "left").unwrap();

    let t = Instant::now();
    let r = s.searchsorted_values(&needles, "left").unwrap();
    let d = t.elapsed();
    assert_eq!(r.len(), m);

    println!(
        "TIMING n={n} m={m} searchsorted_values={:.3}ms",
        d.as_secs_f64() * 1e3
    );
}
