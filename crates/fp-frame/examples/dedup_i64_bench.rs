//! drop_duplicates on a single all-valid Int64 column (with many duplicates),
//! head-to-head measurable vs pandas `df.drop_duplicates(subset=['key'])`.
//! Run: cargo run -p fp-frame --example dedup_i64_bench --release -- 100000 1000 30

use std::{collections::BTreeMap, time::Instant};

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::{DuplicateKeep, Index};

fn best<F: FnMut()>(iters: usize, mut f: F) -> u128 {
    for _ in 0..3 {
        f();
    }
    let mut b = u128::MAX;
    for _ in 0..iters {
        let t = Instant::now();
        f();
        b = b.min(t.elapsed().as_nanos());
    }
    b
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let rows: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(100_000);
    let distinct: i64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(1_000);
    let iters: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(30);

    // splitmix64 keys in [0, distinct) — many duplicates, mirrors a categorical-id column.
    let mut state: u64 = 0x9e37_79b9_7f4a_7c15;
    let mut next = || {
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    };
    let mode = args.get(4).map(String::as_str).unwrap_or("i64");
    let payload: Vec<f64> = (0..rows).map(|i| i as f64).collect();

    let index = Index::new_known_unique_int64_unit_range(0, rows);
    let mut columns = BTreeMap::new();
    if mode == "utf8" {
        let mut kb = Vec::with_capacity(rows * 6);
        let mut ko = Vec::with_capacity(rows + 1);
        ko.push(0usize);
        for _ in 0..rows {
            kb.extend_from_slice(format!("k{:08}", next() % distinct as u64).as_bytes());
            ko.push(kb.len());
        }
        columns.insert("key".to_string(), Column::from_utf8_contiguous(kb, ko));
    } else {
        let keys: Vec<i64> = (0..rows)
            .map(|_| (next() % distinct as u64) as i64)
            .collect();
        columns.insert("key".to_string(), Column::from_i64_values(keys));
    }
    columns.insert("val".to_string(), Column::from_f64_values(payload));
    let df = DataFrame::new_with_column_order(
        index,
        columns,
        vec!["key".to_string(), "val".to_string()],
    )
    .expect("frame");

    let subset = vec!["key".to_string()];
    let ns = best(iters, || {
        let _ = df
            .drop_duplicates(Some(&subset), DuplicateKeep::First, false)
            .expect("drop_duplicates");
    });
    let out = df
        .drop_duplicates(Some(&subset), DuplicateKeep::First, false)
        .unwrap();
    eprintln!(
        "dedup_i64: rows={rows} distinct={distinct} kept={} best={:.1}us",
        out.len(),
        ns as f64 / 1000.0
    );
    println!("{:.1}", ns as f64 / 1000.0);
}
