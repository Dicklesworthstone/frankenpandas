//! Differential golden for Series.value_counts on Float64: proves the output
//! (index labels + counts, in order) is bit-identical to an independent
//! reference (first-seen-order hash tally + stable descending count sort) across
//! all-distinct data (the new sort-based unique fast path), data with
//! duplicates (the hash fallback), and a +0.0/-0.0 canonicalization case.
//!
//! Run: cargo run -p fp-frame --example golden_value_counts_f64 --release

use std::collections::HashMap;

use fp_columnar::Column;
use fp_frame::Series;
use fp_index::{Index, IndexLabel};

fn reference(data: &[f64]) -> Vec<(u64, i64)> {
    // First-seen order hash tally, then STABLE descending sort by count — the
    // exact semantics the production path documents.
    let mut idx: HashMap<u64, usize> = HashMap::new();
    let mut out: Vec<(u64, i64)> = Vec::new();
    for &v in data {
        if v.is_nan() {
            continue;
        }
        let bits = if v == 0.0 { 0 } else { v.to_bits() };
        match idx.get(&bits) {
            Some(&i) => out[i].1 += 1,
            None => {
                idx.insert(bits, out.len());
                out.push((bits, 1));
            }
        }
    }
    out.sort_by_key(|row| std::cmp::Reverse(row.1)); // stable desc by count
    out
}

fn actual(data: &[f64]) -> Vec<(u64, i64)> {
    let labels: Vec<IndexLabel> = (0..data.len() as i64).map(IndexLabel::Int64).collect();
    let s = Series::new(
        "x".to_string(),
        Index::new(labels),
        Column::from_f64_values(data.to_vec()),
    )
    .unwrap();
    let vc = s.value_counts().unwrap();
    let idx_labels = vc.index().labels();
    let counts = vc.column().values();
    idx_labels
        .iter()
        .zip(counts.iter())
        .map(|(lbl, c)| {
            let v = match lbl {
                IndexLabel::Float64(o) => o.0,
                other => panic!("unexpected label {other:?}"),
            };
            let bits = if v == 0.0 { 0 } else { v.to_bits() };
            let cnt = match c {
                fp_types::Scalar::Int64(n) => *n,
                other => panic!("unexpected count {other:?}"),
            };
            (bits, cnt)
        })
        .collect()
}

fn check(name: &str, data: &[f64]) {
    let r = reference(data);
    let a = actual(data);
    if r == a {
        println!("OK   {name}: {} distinct", r.len());
    } else {
        println!("FAIL {name}: ref.len={} act.len={}", r.len(), a.len());
        for i in 0..r.len().max(a.len()) {
            let rr = r.get(i);
            let aa = a.get(i);
            if rr != aa {
                println!("  [{i}] ref={rr:?} act={aa:?}");
                if i > 5 {
                    break;
                }
            }
        }
        std::process::exit(1);
    }
}

fn main() {
    let n = 50_000usize;
    let mut z = 0x9e37_79b9_7f4a_7c15u64;
    let mut next = || {
        z ^= z << 13;
        z ^= z >> 7;
        z ^= z << 17;
        (z >> 11) as f64 / (1u64 << 53) as f64 * 1e6
    };

    // All-distinct (hits the new sort-based unique fast path).
    let distinct: Vec<f64> = (0..n).map(|_| next()).collect();
    check("all_distinct", &distinct);

    // Heavy duplicates (forces the hash-tally fallback, varied counts).
    let dup: Vec<f64> = (0..n).map(|i| (i % 137) as f64).collect();
    check("heavy_dups", &dup);

    // Mixed: mostly distinct with a few repeats (probe may pass; dup-check then
    // forces fallback — exercises the None return after the sort).
    let mut mixed: Vec<f64> = (0..n).map(|_| next()).collect();
    mixed[10] = mixed[20];
    mixed[30] = mixed[40];
    check("mostly_distinct_few_dups", &mixed);

    // +0.0 / -0.0 canonicalization (must merge into one group).
    let mut zeros: Vec<f64> = (0..n).map(|_| next()).collect();
    zeros[0] = 0.0;
    zeros[1] = -0.0;
    check("plus_minus_zero", &zeros);

    // Small (below MIN sample → unique probe declines → hash path).
    check("tiny", &[3.0, 1.0, 3.0, 2.0, 1.0, 3.0]);

    println!("ALL GOLDEN CHECKS PASSED");
}
