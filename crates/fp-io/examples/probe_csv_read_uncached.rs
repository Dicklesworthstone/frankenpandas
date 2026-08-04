//! Probe: TRUE (cache-missing) read_csv_str cost. `read_csv_str` caches by
//! input string, so the vs-pandas harness (re-reading one string) measures
//! cache hits. Here we pre-build K DISTINCT CSVs and read each ONCE.
//! Run: cargo run -p fp-io --example probe_csv_read_uncached --release -- 100000 10 12

use std::{collections::BTreeMap, hint::black_box, time::Instant};

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::{Index, IndexLabel};
use fp_io::{read_csv_str, write_csv_string};

fn build(rows: usize, cols: usize, seed: u64) -> DataFrame {
    let labels: Vec<IndexLabel> = (0..rows as i64).map(IndexLabel::Int64).collect();
    let index = Index::new(labels);
    let mut map = BTreeMap::new();
    let mut order = Vec::with_capacity(cols);
    for c in 0..cols {
        let mut z = seed.wrapping_add((c as u64).wrapping_mul(0x9e3779b97f4a7c15));
        let v: Vec<f64> = (0..rows)
            .map(|_| {
                z ^= z << 13;
                z ^= z >> 7;
                z ^= z << 17;
                (z >> 11) as f64 / (1u64 << 53) as f64 * 1e6
            })
            .collect();
        let name = format!("col_{c}");
        map.insert(name.clone(), Column::from_f64_values(v));
        order.push(name);
    }
    DataFrame::new_with_column_order(index, map, order).expect("frame")
}

fn main() {
    let mut args = std::env::args().skip(1);
    let rows: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(100_000);
    let cols: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(10);
    let k: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(12);

    // Pre-build K distinct CSV strings (so each read_csv_str MISSES the cache).
    let csvs: Vec<String> = (0..k)
        .map(|i| write_csv_string(&build(rows, cols, 0x1000 + i as u64)).expect("write"))
        .collect();
    let mb = csvs[0].len() as f64 / 1e6;

    // Warm (parse the first 2, also primes allocator).
    for c in csvs.iter().take(2) {
        black_box(read_csv_str(c).expect("read"));
    }
    let start = Instant::now();
    let mut sink = 0usize;
    for c in &csvs {
        let df = black_box(read_csv_str(c).expect("read"));
        sink ^= df.len();
    }
    let per = start.elapsed().as_secs_f64() * 1000.0 / k as f64;
    println!(
        "uncached read_csv_str rows={rows} cols={cols}: {per:.2} ms/read ({mb:.1} MB, {:.0} MB/s, sink={sink})",
        mb / (per / 1000.0)
    );
}
