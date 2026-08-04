//! Bench + golden digest for Series/DataFrame::sort_index over an Int64 index.
//!
//! Run: cargo run -p fp-frame --example bench_sort_index --release
//!
//! sort_index used an O(n log n) comparison sort on IndexLabel. An all-Int64
//! index sorts by raw i64, so Column's stable typed radix argsort (O(n)) is
//! bit-identical — including stable tie order for duplicate labels.

use std::{collections::BTreeMap, time::Instant};

use fp_columnar::Column;
use fp_frame::{DataFrame, Series};
use fp_index::{Index, IndexLabel};
use fp_types::Scalar;

fn series(labels: Vec<i64>, vals: Vec<i64>) -> Series {
    let idx: Vec<IndexLabel> = labels.into_iter().map(IndexLabel::Int64).collect();
    let sc: Vec<Scalar> = vals.into_iter().map(Scalar::Int64).collect();
    Series::from_values("s", idx, sc).unwrap()
}

fn frame(labels: Vec<i64>, vals: Vec<i64>) -> DataFrame {
    let idx = Index::new(labels.into_iter().map(IndexLabel::Int64).collect());
    let mut cols = BTreeMap::new();
    cols.insert(
        "v".to_string(),
        Column::from_values(vals.into_iter().map(Scalar::Int64).collect()).unwrap(),
    );
    DataFrame::new_with_column_order(idx, cols, vec!["v".to_string()]).unwrap()
}

fn golden() -> String {
    let mut out = String::new();
    // Duplicate labels 1 and 3 with distinct values prove stable tie order.
    let s = series(vec![3, 1, 2, 1, 3], vec![10, 20, 30, 40, 50]);
    let asc = s.sort_index(true).unwrap();
    out.push_str(&format!("s_asc_labels={:?}\n", asc.index().labels()));
    out.push_str(&format!("s_asc_vals={:?}\n", asc.values()));
    let desc = s.sort_index(false).unwrap();
    out.push_str(&format!("s_desc_labels={:?}\n", desc.index().labels()));
    out.push_str(&format!("s_desc_vals={:?}\n", desc.values()));

    // Negative + zero labels.
    let s2 = series(vec![0, -5, 7, -5, 0], vec![1, 2, 3, 4, 5]);
    let a2 = s2.sort_index(true).unwrap();
    out.push_str(&format!("s2_asc_labels={:?}\n", a2.index().labels()));
    out.push_str(&format!("s2_asc_vals={:?}\n", a2.values()));

    let df = frame(vec![3, 1, 2, 1, 3], vec![10, 20, 30, 40, 50]);
    let dfa = df.sort_index(true).unwrap();
    out.push_str(&format!("df_asc_labels={:?}\n", dfa.index().labels()));
    out.push_str(&format!(
        "df_asc_v={:?}\n",
        dfa.columns().get("v").unwrap().values()
    ));
    let dfd = df.sort_index(false).unwrap();
    out.push_str(&format!("df_desc_labels={:?}\n", dfd.index().labels()));
    out.push_str(&format!(
        "df_desc_v={:?}\n",
        dfd.columns().get("v").unwrap().values()
    ));
    out
}

fn main() {
    let g = golden();
    print!("GOLDEN_BEGIN\n{g}GOLDEN_END\n");

    let n: usize = 200_000;
    // Shuffled (LCG) i64 index so it is unsorted and Int64-typed.
    let mut x: u64 = 0x1234_5678;
    let labels: Vec<i64> = (0..n)
        .map(|_| {
            x = x
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (x >> 16) as i64 % (n as i64)
        })
        .collect();
    let vals: Vec<i64> = (0..n as i64).collect();
    let s = series(labels, vals);

    // warmup
    let _ = s.sort_index(true).unwrap();

    let t = Instant::now();
    let r = s.sort_index(true).unwrap();
    let d = t.elapsed();
    assert_eq!(r.len(), n);

    println!("TIMING n={n} sort_index={:.3}ms", d.as_secs_f64() * 1e3);
}
