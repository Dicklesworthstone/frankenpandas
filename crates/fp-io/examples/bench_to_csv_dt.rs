//! to_csv over a 1M-row DataFrame with a Datetime64 column. bench_to_csv_dt <n>
use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::Index;
use std::collections::BTreeMap;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(1_000_000);
    let base = 946_684_800_000_000_000i64; // 2000-01-01
    let step = 37_000_000_000i64; // 37s (whole seconds, no subsec)
    let nanos: Vec<i64> = (0..n as i64).map(|i| base + i * step).collect();
    let mut cols: BTreeMap<String, Column> = BTreeMap::new();
    cols.insert("t".to_string(), Column::from_datetime64_values(nanos));
    let frame = DataFrame::new(Index::new_known_unique_int64_unit_range(0, n), cols).unwrap();
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
    println!("to_csv datetime n={n}: best={best}ns ({:.2}ms)", best as f64 / 1e6);
}
