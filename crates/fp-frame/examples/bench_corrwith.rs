//! Bench + golden digest for DataFrame::corrwith_axis(other, axis=1).
//!
//! Run: cargo run -p fp-frame --example bench_corrwith --release
//!
//! Row-wise corrwith matched each self-row to other by scanning other's index
//! with `Index::position` — a linear scan for any non-ascending-Int64 index,
//! i.e. O(n·m). A first-occurrence label->row map built once makes it O(n).
//! Bit-identical: position_map_first records the FIRST occurrence, exactly
//! what Index::position returns; unmatched rows are still dropped.

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::{Index, IndexLabel};
use fp_types::Scalar;
use std::collections::BTreeMap;
use std::time::Instant;

fn frame(labels: Vec<IndexLabel>, cols: Vec<(&str, Vec<f64>)>) -> DataFrame {
    let order: Vec<String> = cols.iter().map(|(n, _)| (*n).to_string()).collect();
    let mut map = BTreeMap::new();
    for (n, vs) in cols {
        map.insert(
            n.to_string(),
            Column::from_values(vs.into_iter().map(Scalar::Float64).collect()).unwrap(),
        );
    }
    DataFrame::new_with_column_order(Index::new(labels), map, order).unwrap()
}

fn lbl(s: &str) -> IndexLabel {
    IndexLabel::Utf8(s.to_string())
}

fn golden() -> String {
    // self has rows r0,r1,r2,r3 ; other has r1 (dup, first wins), r0, r3, rX.
    let s = frame(
        vec![lbl("r0"), lbl("r1"), lbl("r2"), lbl("r3")],
        vec![
            ("a", vec![1.0, 2.0, 3.0, 4.0]),
            ("b", vec![2.0, 1.0, 5.0, 4.0]),
            ("c", vec![9.0, 8.0, 7.0, 1.0]),
        ],
    );
    let o = frame(
        vec![lbl("r1"), lbl("r0"), lbl("r3"), lbl("r1"), lbl("rX")],
        vec![
            ("a", vec![5.0, 1.5, 4.0, -9.0, 0.0]),
            ("b", vec![6.0, 2.5, 3.0, -9.0, 0.0]),
            ("c", vec![1.0, 0.5, 8.0, -9.0, 0.0]),
        ],
    );
    let r = s.corrwith_axis(&o, 1).unwrap();
    let mut out = String::new();
    out.push_str(&format!("labels={:?}\n", r.index().labels()));
    // Round values to 9 decimals for a stable digest.
    let vals: Vec<String> = r
        .column()
        .values()
        .iter()
        .map(|v| match v {
            Scalar::Float64(f) if f.is_nan() => "nan".to_string(),
            Scalar::Float64(f) => format!("{:.9}", f),
            other => format!("{other:?}"),
        })
        .collect();
    out.push_str(&format!("values={:?}\n", vals));
    out
}

fn main() {
    let g = golden();
    print!("GOLDEN_BEGIN\n{g}GOLDEN_END\n");

    // Large non-ascending-Int64 (string) index so position() is linear.
    let n: usize = 20_000;
    let labels: Vec<IndexLabel> = (0..n).map(|i| lbl(&format!("k{i:08}"))).collect();
    let mkcol = |mult: f64| (0..n).map(|i| (i as f64) * mult).collect::<Vec<f64>>();
    let s = frame(
        labels.clone(),
        vec![
            ("a", mkcol(1.0)),
            ("b", mkcol(2.0)),
            ("c", mkcol(0.5)),
            ("d", mkcol(3.0)),
        ],
    );
    // other: same labels, reversed order (so a sorted shortcut can't apply).
    let mut rlabels = labels.clone();
    rlabels.reverse();
    let o = frame(
        rlabels,
        vec![
            ("a", mkcol(1.1)),
            ("b", mkcol(2.2)),
            ("c", mkcol(0.7)),
            ("d", mkcol(3.3)),
        ],
    );

    // warmup
    let _ = s.corrwith_axis(&o, 1).unwrap();

    let t = Instant::now();
    let r = s.corrwith_axis(&o, 1).unwrap();
    let d = t.elapsed();
    assert_eq!(r.len(), n);

    println!("TIMING n={n} corrwith_axis1={:.3}ms", d.as_secs_f64() * 1e3);
}
