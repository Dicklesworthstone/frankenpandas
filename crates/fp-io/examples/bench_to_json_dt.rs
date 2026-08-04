//! to_json(records) over a frame with a Datetime64 column. bench_to_json_dt <n>
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
    let base = 946_684_800_000_000_000i64;
    let step = 37_000_000_000i64;
    let dts: Vec<i64> = (0..n as i64).map(|i| base + i * step).collect();
    let mut cols: BTreeMap<String, Column> = BTreeMap::new();
    cols.insert("a".to_string(), Column::from_i64_values(ints));
    cols.insert("t".to_string(), Column::from_datetime64_values(dts));
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
        "to_json(records) +dt n={n}: best={best}ns ({:.2}ms)",
        best as f64 / 1e6
    );
}
