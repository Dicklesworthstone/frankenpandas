//! Bench + golden for write_csv_string — hoist per-cell column lookup.
//!
//! Run: cargo run -p fp-io --example bench_to_csv --release
//!
//! The row loop did a `frame.column(name)` BTreeMap-by-name lookup per CELL
//! (O(N·C·log C)); resolving the column refs once before the loop makes it
//! O(N·C). Output is byte-identical.

use std::{collections::BTreeMap, time::Instant};

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::{Index, IndexLabel};
use fp_io::write_csv_string;
use fp_types::{NullKind, Scalar};

fn frame(n_rows: usize, n_cols: usize, salt: i64) -> DataFrame {
    let labels: Vec<IndexLabel> = (0..n_rows as i64).map(IndexLabel::Int64).collect();
    let mut order = Vec::new();
    let mut cols = BTreeMap::new();
    for c in 0..n_cols {
        let name = format!("col{c:03}");
        let vals: Vec<Scalar> = (0..n_rows as i64)
            .map(|i| {
                if (i + c as i64) % 17 == 0 {
                    Scalar::Null(NullKind::NaN)
                } else if c % 2 == 0 {
                    Scalar::Int64(i.wrapping_mul(salt + c as i64).wrapping_add(1))
                } else {
                    Scalar::Float64((i as f64) * 0.5 - (c as f64))
                }
            })
            .collect();
        cols.insert(name.clone(), Column::from_values(vals).unwrap());
        order.push(name);
    }
    DataFrame::new_with_column_order(Index::new(labels), cols, order).unwrap()
}

fn golden() -> String {
    // Small mixed frame incl nulls, ints, floats, strings.
    let labels: Vec<IndexLabel> = (0..4i64).map(IndexLabel::Int64).collect();
    let mut cols = BTreeMap::new();
    cols.insert(
        "a".to_string(),
        Column::from_values(vec![
            Scalar::Int64(1),
            Scalar::Null(NullKind::NaN),
            Scalar::Int64(-3),
            Scalar::Int64(42),
        ])
        .unwrap(),
    );
    cols.insert(
        "b".to_string(),
        Column::from_values(vec![
            Scalar::Float64(1.5),
            Scalar::Float64(-0.0),
            Scalar::Float64(2.25),
            Scalar::Null(NullKind::NaN),
        ])
        .unwrap(),
    );
    cols.insert(
        "c".to_string(),
        Column::from_values(
            ["x", "y,z", "", "q\"q"]
                .iter()
                .map(|s| Scalar::Utf8((*s).to_string()))
                .collect(),
        )
        .unwrap(),
    );
    let df = DataFrame::new_with_column_order(
        Index::new(labels),
        cols,
        vec!["a".into(), "b".into(), "c".into()],
    )
    .unwrap();
    write_csv_string(&df).unwrap()
}

fn main() {
    let g = golden();
    print!("GOLDEN_BEGIN\n{g}GOLDEN_END\n");

    // Wide frame: many columns => the per-cell BTreeMap lookup dominates.
    let df = frame(40_000, 60, 7);

    let _ = write_csv_string(&df).unwrap(); // warmup

    let t = Instant::now();
    let s = write_csv_string(&df).unwrap();
    let d = t.elapsed();
    std::hint::black_box(s.len());

    println!(
        "TIMING rows=40000 cols=60 to_csv={:.3}ms",
        d.as_secs_f64() * 1e3
    );
}
