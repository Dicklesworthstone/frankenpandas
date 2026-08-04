//! to_json(records) over a frame with a Utf8 column. bench_to_json_str <n>
use std::collections::BTreeMap;

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::Index;
use fp_io::JsonOrient;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(1_000_000);
    let ints: Vec<i64> = (0..n as i64)
        .map(|i| (i.wrapping_mul(2_654_435_761)) % 1_000_000)
        .collect();
    // Contiguous Utf8 backing (what read_csv / string ops produce).
    let mut sbytes: Vec<u8> = Vec::new();
    let mut soff: Vec<usize> = vec![0];
    for i in 0..n {
        sbytes.extend_from_slice(format!("item_{}", i % 1000).as_bytes());
        soff.push(sbytes.len());
    }
    let mut cols: BTreeMap<String, Column> = BTreeMap::new();
    cols.insert("a".to_string(), Column::from_i64_values(ints));
    cols.insert("s".to_string(), Column::from_utf8_contiguous(sbytes, soff));
    let frame = DataFrame::new(Index::new_known_unique_int64_unit_range(0, n), cols).unwrap();
    let mut best = u128::MAX;
    for _ in 0..6 {
        let t = std::time::Instant::now();
        let s = fp_io::write_json_string(&frame, JsonOrient::Records).unwrap();
        std::hint::black_box(s.len());
        let e = t.elapsed().as_nanos();
        if e < best {
            best = e;
        }
    }
    println!(
        "to_json(records) +str n={n}: best={best}ns ({:.2}ms)",
        best as f64 / 1e6
    );
}
