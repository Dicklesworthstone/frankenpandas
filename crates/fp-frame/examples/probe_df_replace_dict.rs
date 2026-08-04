//! Probe: DataFrame.replace_dict on an all-Int64 frame with per-column maps.
//! Run: cargo run -p fp-frame --example probe_df_replace_dict --release -- 200000 10 50

use std::{collections::BTreeMap, hint::black_box, time::Instant};

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::{Index, IndexLabel};
use fp_types::Scalar;

fn main() {
    let mut a = std::env::args().skip(1);
    let n: usize = a.next().and_then(|s| s.parse().ok()).unwrap_or(200_000);
    let cols: usize = a.next().and_then(|s| s.parse().ok()).unwrap_or(10);
    let m: i64 = a.next().and_then(|s| s.parse().ok()).unwrap_or(50);

    let labels: Vec<IndexLabel> = (0..n as i64).map(IndexLabel::Int64).collect();
    let index = Index::new(labels);
    let mut map = BTreeMap::new();
    let mut order = Vec::new();
    let mut z = 0x1234u64;
    for c in 0..cols {
        let v: Vec<i64> = (0..n)
            .map(|_| {
                z ^= z << 13;
                z ^= z >> 7;
                z ^= z << 17;
                (z % 100) as i64
            })
            .collect();
        let name = format!("col_{c}");
        map.insert(name.clone(), Column::from_i64_values(v));
        order.push(name);
    }
    let df = DataFrame::new_with_column_order(index, map, order).unwrap();
    let per_column: BTreeMap<String, Vec<(Scalar, Scalar)>> = (0..cols)
        .map(|c| {
            let repl: Vec<(Scalar, Scalar)> = (0..m)
                .map(|i| (Scalar::Int64(i), Scalar::Int64(i + 1000 + c as i64)))
                .collect();
            (format!("col_{c}"), repl)
        })
        .collect();

    for _ in 0..2 {
        black_box(df.replace_dict(&per_column).unwrap());
    }
    let it = 5;
    let start = Instant::now();
    let mut sink = 0usize;
    for _ in 0..it {
        sink ^= black_box(df.replace_dict(&per_column).unwrap()).len();
    }
    println!(
        "DataFrame.replace_dict n={n} cols={cols} m={m}: {:.2} ms/call (sink={sink})",
        start.elapsed().as_secs_f64() * 1000.0 / it as f64
    );
}
