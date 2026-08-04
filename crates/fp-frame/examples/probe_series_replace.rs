//! Probe: Series.replace(&[(Scalar,Scalar)]) on an Int64 series with an
//! Int64->Int64 replacement table. Run:
//!   cargo run -p fp-frame --example probe_series_replace --release -- 1000000 100

use std::{hint::black_box, time::Instant};

use fp_columnar::Column;
use fp_frame::Series;
use fp_index::{Index, IndexLabel};
use fp_types::Scalar;

fn main() {
    let mut a = std::env::args().skip(1);
    let n: usize = a.next().and_then(|s| s.parse().ok()).unwrap_or(1_000_000);
    let cats: i64 = a.next().and_then(|s| s.parse().ok()).unwrap_or(100);
    // Replace only HALF the categories (so half the rows keep their original).
    let repl_cats = cats / 2;

    let mut z = 0x1234u64;
    let vals: Vec<i64> = (0..n)
        .map(|_| {
            z ^= z << 13;
            z ^= z >> 7;
            z ^= z << 17;
            (z % cats as u64) as i64
        })
        .collect();
    let labels: Vec<IndexLabel> = (0..n as i64).map(IndexLabel::Int64).collect();
    let s = Series::new(
        "c".to_string(),
        Index::new(labels),
        Column::from_i64_values(vals),
    )
    .unwrap();
    let repl: Vec<(Scalar, Scalar)> = (0..repl_cats)
        .map(|i| (Scalar::Int64(i), Scalar::Int64(i + 10_000)))
        .collect();

    for _ in 0..2 {
        black_box(s.replace(&repl).unwrap());
    }
    let it = 8;
    let start = Instant::now();
    let mut sink = 0usize;
    for _ in 0..it {
        sink ^= black_box(s.replace(&repl).unwrap()).len();
    }
    println!(
        "Series.replace n={n} cats={cats}: {:.2} ms/call (sink={sink})",
        start.elapsed().as_secs_f64() * 1000.0 / it as f64
    );
}
