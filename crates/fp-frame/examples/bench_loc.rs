//! Bench + golden digest for Series::loc(&[IndexLabel]) (series.loc[[...]]).
//!
//! Run: cargo run -p fp-frame --example bench_loc --release
//!
//! loc scanned the whole index once PER requested label = O(m·n). A
//! label->positions multimap built once makes it O(m+n). Duplicate index
//! labels return all matches in ascending index order; selector order and
//! duplicate selectors are preserved; a missing label fails closed.

use std::time::Instant;

use fp_frame::Series;
use fp_index::IndexLabel;
use fp_types::Scalar;

fn s_from(labels: Vec<i64>, vals: Vec<i64>) -> Series {
    let idx: Vec<IndexLabel> = labels.into_iter().map(IndexLabel::Int64).collect();
    let sc: Vec<Scalar> = vals.into_iter().map(Scalar::Int64).collect();
    Series::from_values("s", idx, sc).unwrap()
}

fn golden() -> String {
    let mut out = String::new();
    // Duplicate index label 10 at positions 0 and 2.
    let s = s_from(vec![10, 20, 10, 30], vec![100, 200, 300, 400]);
    // Selector order [10, 30, 20]; 10 returns both matches (asc position).
    let r = s
        .loc(&[
            IndexLabel::Int64(10),
            IndexLabel::Int64(30),
            IndexLabel::Int64(20),
        ])
        .unwrap();
    out.push_str(&format!("labels={:?}\n", r.index().labels()));
    out.push_str(&format!("values={:?}\n", r.values()));
    // Duplicate selector entries preserved.
    let r2 = s
        .loc(&[IndexLabel::Int64(20), IndexLabel::Int64(20)])
        .unwrap();
    out.push_str(&format!("dup_sel_values={:?}\n", r2.values()));
    // Missing label fails closed.
    let err = s.loc(&[IndexLabel::Int64(99)]);
    out.push_str(&format!("missing_is_err={}\n", err.is_err()));
    out
}

fn main() {
    let g = golden();
    print!("GOLDEN_BEGIN\n{g}GOLDEN_END\n");

    let n: usize = 60_000;
    let labels: Vec<i64> = (0..n as i64).collect();
    let vals: Vec<i64> = (0..n as i64).map(|v| v * 2).collect();
    let s = s_from(labels.clone(), vals);
    let selector: Vec<IndexLabel> = (0..n as i64).map(IndexLabel::Int64).collect();

    let t = Instant::now();
    let r = s.loc(&selector).unwrap();
    let d = t.elapsed();
    assert_eq!(r.len(), n);

    println!("TIMING n={n} m={n} loc={:.3}ms", d.as_secs_f64() * 1e3);
}
