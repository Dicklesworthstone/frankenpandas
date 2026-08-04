//! Phase-level timing for Series.value_counts on an all-distinct Float64 column,
//! to locate the hotspot vs pandas. Run:
//!   cargo run -p fp-frame --example prof_value_counts --release -- 1000000

use std::time::Instant;

use fp_columnar::Column;
use fp_frame::Series;
use fp_index::{Index, IndexLabel};

fn main() {
    let n: usize = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(1_000_000);
    let mut z = 0x1234_5678_9abc_def0u64;
    let data: Vec<f64> = (0..n)
        .map(|_| {
            z ^= z << 13;
            z ^= z >> 7;
            z ^= z << 17;
            (z >> 11) as f64 / (1u64 << 53) as f64 * 1e6
        })
        .collect();
    let labels: Vec<IndexLabel> = (0..n as i64).map(IndexLabel::Int64).collect();
    let s = Series::new(
        "col_0".to_string(),
        Index::new(labels),
        Column::from_f64_values(data.clone()),
    )
    .expect("series");

    // Warm + time full value_counts.
    for _ in 0..3 {
        let _ = s.value_counts().unwrap();
    }
    let reps = 10;
    let t0 = Instant::now();
    for _ in 0..reps {
        let _ = s.value_counts().unwrap();
    }
    let full = t0.elapsed().as_secs_f64() * 1e6 / reps as f64;

    // Phase A: raw FxHashMap-by-bits tally only.
    let t1 = Instant::now();
    let mut distinct = 0usize;
    for _ in 0..reps {
        let mut idx: std::collections::HashMap<u64, usize> =
            std::collections::HashMap::with_capacity(data.len());
        let mut out: Vec<(u64, f64, usize)> = Vec::with_capacity(data.len());
        for &v in &data {
            let bits = if v == 0.0 { 0 } else { v.to_bits() };
            match idx.get(&bits) {
                Some(&i) => out[i].2 += 1,
                None => {
                    idx.insert(bits, out.len());
                    out.push((bits, v, 1));
                }
            }
        }
        distinct = out.len();
    }
    let tally = t1.elapsed().as_secs_f64() * 1e6 / reps as f64;

    println!("n={n} distinct={distinct}");
    println!("full value_counts : {full:.0} us");
    println!(
        "std-hashmap tally : {tally:.0} us  ({:.0}% of full)",
        100.0 * tally / full
    );
    println!(
        "rest (sort+materialize+index+column): {:.0} us",
        full - tally
    );
}
