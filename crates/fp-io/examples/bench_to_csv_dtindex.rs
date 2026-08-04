//! to_csv (with index) over a DatetimeIndex + numeric columns. bench_to_csv_dtindex <n>
use std::collections::BTreeMap;

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::{Index, IndexLabel};

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(1_000_000);
    let base = 946_684_800_000_000_000i64;
    let step = 37_000_000_000i64;
    let labels: Vec<IndexLabel> = (0..n as i64)
        .map(|i| IndexLabel::Datetime64(base + i * step))
        .collect();
    let ints: Vec<i64> = (0..n as i64)
        .map(|i| (i.wrapping_mul(2_654_435_761)) % 1_000_000)
        .collect();
    let floats: Vec<f64> = (0..n).map(|i| (i as f64) * 0.5 - 100.0).collect();
    let mut cols: BTreeMap<String, Column> = BTreeMap::new();
    cols.insert("a".to_string(), Column::from_i64_values(ints));
    cols.insert("b".to_string(), Column::from_f64_values(floats));
    let frame = DataFrame::new(Index::new(labels), cols).unwrap();
    let mut best = u128::MAX;
    for _ in 0..6 {
        let t = std::time::Instant::now();
        let s = fp_io::write_csv_string(&frame).unwrap();
        std::hint::black_box(s.len());
        let e = t.elapsed().as_nanos();
        if e < best {
            best = e;
        }
    }
    println!(
        "to_csv dtindex n={n}: best={best}ns ({:.2}ms)",
        best as f64 / 1e6
    );
}
