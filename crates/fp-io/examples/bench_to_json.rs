//! to_json(records) over a mixed-dtype frame. bench_to_json <n>
use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::Index;
use fp_io::JsonOrient;
use std::collections::BTreeMap;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(1_000_000);
    let ints: Vec<i64> = (0..n as i64).map(|i| (i.wrapping_mul(2_654_435_761)) % 1_000_000).collect();
    let floats: Vec<f64> = (0..n).map(|i| (i as f64) * 0.5 - 100.0).collect();
    let orient = match a.get(2).map(String::as_str).unwrap_or("records") {
        "values" => JsonOrient::Values,
        "split" => JsonOrient::Split,
        "columns" => JsonOrient::Columns,
        "index" => JsonOrient::Index,
        _ => JsonOrient::Records,
    };
    let mut cols: BTreeMap<String, Column> = BTreeMap::new();
    cols.insert("a".to_string(), Column::from_i64_values(ints));
    cols.insert("b".to_string(), Column::from_f64_values(floats));
    let frame = DataFrame::new(Index::new_known_unique_int64_unit_range(0, n), cols).unwrap();
    let mut best = u128::MAX;
    for _ in 0..6 {
        let t = std::time::Instant::now();
        let s = fp_io::write_json_string(&frame, orient).unwrap();
        std::hint::black_box(s.len());
        let e = t.elapsed().as_nanos();
        if e < best {
            best = e;
        }
    }
    println!("to_json({orient:?}) n={n}: best={best}ns ({:.2}ms)", best as f64 / 1e6);
}
