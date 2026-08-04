//! Bench + golden for DataFrame iloc/take/sample — typed gather lever.
//!
//! Run: cargo run -p fp-frame --example bench_df_iloc_gather --release
//!
//! DataFrame iloc_with_columns, take_rows and sample gathered each column with
//! a per-row Scalar clone + Column::new/from_values. Routing through the typed
//! Column::take_positions keeps the contiguous Int64/Float64 buffer. Output is
//! bit-identical (values, dtype, negative/duplicate indices, NaN).

use std::{collections::BTreeMap, time::Instant};

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::{Index, IndexLabel};
use fp_types::Scalar;

fn frame(labels: Vec<i64>, a: Vec<i64>, b: Vec<Scalar>) -> DataFrame {
    let idx = Index::new(labels.into_iter().map(IndexLabel::Int64).collect());
    let mut cols = BTreeMap::new();
    cols.insert(
        "a".to_string(),
        Column::from_values(a.into_iter().map(Scalar::Int64).collect()).unwrap(),
    );
    cols.insert("b".to_string(), Column::from_values(b).unwrap());
    DataFrame::new_with_column_order(idx, cols, vec!["a".to_string(), "b".to_string()]).unwrap()
}

fn dump(df: &DataFrame) -> String {
    format!(
        "lbls={:?} a={:?} b={:?}",
        df.index().labels(),
        df.columns().get("a").unwrap().values(),
        df.columns().get("b").unwrap().values(),
    )
}

fn golden() -> String {
    let mut out = String::new();
    let df = frame(
        vec![10, 11, 12, 13, 14],
        vec![100, 200, 300, 400, 500],
        vec![
            Scalar::Float64(1.5),
            Scalar::Float64(f64::NAN),
            Scalar::Float64(-3.0),
            Scalar::Float64(2.0),
            Scalar::Float64(9.0),
        ],
    );

    // iloc: reorder + negative + duplicate
    out.push_str(&format!(
        "iloc={}\n",
        dump(&df.iloc(&[4, 0, -1, 2, 2]).unwrap())
    ));
    out.push_str(&format!("iloc_oob_err={}\n", df.iloc(&[99]).is_err()));
    // take axis=0
    out.push_str(&format!(
        "take={}\n",
        dump(&df.take(&[3, 1, 0], 0).unwrap())
    ));
    out.push_str(&format!("take_oob_err={}\n", df.take(&[99], 0).is_err()));
    // sample deterministic (fixed seed)
    out.push_str(&format!(
        "sample={}\n",
        dump(&df.sample(Some(3), None, false, Some(7)).unwrap())
    ));
    out
}

fn main() {
    let g = golden();
    print!("GOLDEN_BEGIN\n{g}GOLDEN_END\n");

    let n: usize = 500_000;
    let df = frame(
        (0..n as i64).collect(),
        (0..n as i64).map(|v| v * 2).collect(),
        (0..n).map(|v| Scalar::Float64(v as f64 * 0.5)).collect(),
    );
    let mut x: u64 = 0xabcd_1234;
    let pos: Vec<i64> = (0..n)
        .map(|_| {
            x = x
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (x >> 16) as i64 % (n as i64)
        })
        .collect();

    let _ = df.iloc(&pos).unwrap(); // warmup

    let t = Instant::now();
    let r = df.iloc(&pos).unwrap();
    let d = t.elapsed();
    assert_eq!(r.len(), n);

    println!("TIMING n={n} df_iloc={:.3}ms", d.as_secs_f64() * 1e3);
}
