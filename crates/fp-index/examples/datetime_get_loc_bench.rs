//! Micro-benchmark for `DatetimeIndex::get_loc` (br-frankenpandas-idxdup).
//! get_loc delegates to Index::position, which binary-searches a monotonic
//! (AscendingDatetime64) index in O(log n) instead of the previous O(n) linear
//! scan. A reversed (strictly-descending) index is detected as Unsorted and
//! keeps the linear path — the baseline for what every datetime get_loc did
//! before.

use std::{hint::black_box, time::Instant};

use fp_index::DatetimeIndex;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let n: usize = args
        .get(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(1_000_000);
    let queries: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(2_000);

    let sorted: Vec<i64> = (0..n).map(|i| i as i64 * 1000).collect();
    let reversed: Vec<i64> = (0..n).map(|i| (n - 1 - i) as i64 * 1000).collect();
    let dt_sorted = DatetimeIndex::new(sorted);
    let dt_unsorted = DatetimeIndex::new(reversed);

    // Spread of present query values.
    let qs: Vec<i64> = (0..queries)
        .map(|k| ((k * (n.max(1) / queries.max(1))) as i64) * 1000)
        .collect();

    let mode = args.get(3).map(String::as_str).unwrap_or("sorted");
    let target = if mode == "linear" {
        &dt_unsorted
    } else {
        &dt_sorted
    };

    let mut sink = 0usize;
    let start = Instant::now();
    for &q in &qs {
        sink ^= black_box(target.get_loc(q)).unwrap_or(usize::MAX);
    }
    let elapsed = start.elapsed();
    println!(
        "datetime_get_loc n={n} queries={queries} mode={mode}: {:.3} us/query (sink={sink})",
        elapsed.as_secs_f64() * 1e6 / qs.len() as f64
    );
}
