//! Diagnostic: localize the iloc[a:b] residual cost (index vs columns).
//! Times `df.iloc(n/4..3n/4)` for varying column counts at fixed n. If per-call
//! time scales with #columns, the column gather dominates (columns not lazy);
//! if flat in #columns, the index materialization dominates.
use std::{collections::BTreeMap, time::Instant};

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::{Index, IndexLabel};
use fp_types::Scalar;

fn build(n: usize, cols: usize) -> DataFrame {
    let labels: Vec<IndexLabel> = (0..n).map(|i| IndexLabel::Int64(i as i64)).collect();
    let index = Index::new(labels);
    let mut columns = BTreeMap::new();
    let mut order = Vec::with_capacity(cols);
    for c in 0..cols {
        let name = format!("c{c}");
        let vals: Vec<Scalar> = (0..n)
            .map(|i| Scalar::Float64((i * (c + 1)) as f64 * 0.1))
            .collect();
        columns.insert(name.clone(), Column::from_values(vals).expect("col"));
        order.push(name);
    }
    DataFrame::new_with_column_order(index, columns, order).expect("frame")
}

fn time_iloc(n: usize, cols: usize, iters: usize) -> f64 {
    let frame = build(n, cols);
    let positions: Vec<i64> = ((n / 4) as i64..(3 * n / 4) as i64).collect();
    // warmup (also primes any cached index view)
    for _ in 0..3 {
        let _ = frame.iloc(&positions).expect("iloc");
    }
    let t = Instant::now();
    let mut sink = 0usize;
    for _ in 0..iters {
        let out = frame.iloc(&positions).expect("iloc");
        sink = sink.wrapping_add(out.len());
    }
    let us = t.elapsed().as_secs_f64() * 1e6 / iters as f64;
    std::hint::black_box(sink);
    us
}

fn main() {
    let n = 1_000_000;
    for &cols in &[1usize, 2, 5, 10] {
        let us = time_iloc(n, cols, 50);
        println!(
            "n={n} cols={cols:2} -> {us:8.1} us/call  ({:.1} us/col)",
            us / cols as f64
        );
    }
    // also vary n at fixed 1 col to confirm O(n) in the index path
    for &n in &[100_000usize, 1_000_000] {
        let us = time_iloc(n, 1, 50);
        println!("cols=1 n={n:8} -> {us:8.1} us/call");
    }
}
