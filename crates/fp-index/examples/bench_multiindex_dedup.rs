//! Bench + golden digest for MultiIndex::duplicated/drop_duplicates/nunique/value_counts
//! (br-frankenpandas-midedup).
//!
//! Run: cargo run -p fp-index --example bench_multiindex_dedup --profile release-perf -- <n>
//!
//! The packed-key path hashes one identity-coded u64 per row instead of
//! allocating a Vec<IndexLabel> (Utf8 String clone) per row. `FP_NO_MIDEDUP=1`
//! forces the Vec<IndexLabel>-key baseline; the printed `chk` (FNV digest of the
//! duplicated mask + kept count) must match between the two runs.

use std::{hint::black_box, time::Instant};

use fp_index::{DuplicateKeep, IndexLabel, MultiIndex};
use rustc_hash::FxHashMap;

fn generic_value_counts(mi: &MultiIndex) -> Vec<(Vec<IndexLabel>, usize)> {
    let mut counts: FxHashMap<Vec<IndexLabel>, usize> =
        FxHashMap::with_capacity_and_hasher(mi.len(), Default::default());
    for tuple in mi.to_list() {
        *counts.entry(tuple).or_insert(0) += 1;
    }
    let mut pairs: Vec<(Vec<IndexLabel>, usize)> = counts.into_iter().collect();
    pairs.sort_by(|(left_tuple, left_count), (right_tuple, right_count)| {
        right_count
            .cmp(left_count)
            .then_with(|| left_tuple.cmp(right_tuple))
    });
    pairs
}

fn percentile_ns(samples: &mut [u128], numerator: usize, denominator: usize) -> u128 {
    samples.sort_unstable();
    let Some(last_index) = samples.len().checked_sub(1) else {
        return 0;
    };
    let index = last_index * numerator / denominator;
    samples.get(index).copied().unwrap_or_default()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let n: usize = args
        .get(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(1_000_000);
    let iters: usize = args
        .get(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(10)
        .max(1);

    // ~50% duplicate tuples: low-cardinality 2 Utf8 levels.
    let l0: Vec<IndexLabel> = (0..n)
        .map(|i| IndexLabel::Utf8(format!("g{}", i % 700)))
        .collect();
    let l1: Vec<IndexLabel> = (0..n)
        .map(|i| IndexLabel::Utf8(format!("h{}", (i / 7) % 700)))
        .collect();
    let mi = MultiIndex::from_arrays(vec![l0, l1]).expect("mi");

    let mask = mi.duplicated(DuplicateKeep::First);
    let kept = mi.drop_duplicates().len();
    let nunique = mi.nunique();
    let unique_len = mi.unique().len();
    let reference_counts = generic_value_counts(&mi);
    let candidate_counts = mi.value_counts();
    assert_eq!(candidate_counts, reference_counts);
    if nunique != unique_len {
        eprintln!("nunique mismatch: nunique={nunique} unique_len={unique_len}");
        std::process::exit(2);
    }
    let mut chk: u64 = 0xcbf29ce484222325;
    for &b in &mask {
        chk = (chk ^ u64::from(b)).wrapping_mul(0x100000001b3);
    }
    chk ^= u64::try_from(kept)
        .unwrap_or(u64::MAX)
        .wrapping_mul(0x9E3779B97F4A7C15);
    chk ^= u64::try_from(nunique)
        .unwrap_or(u64::MAX)
        .wrapping_mul(0xBF58476D1CE4E5B9);
    for (tuple, count) in &candidate_counts {
        chk ^= u64::try_from(tuple.len())
            .unwrap_or(u64::MAX)
            .wrapping_mul(0x94D049BB133111EB);
        chk = chk
            .rotate_left(9)
            .wrapping_add(u64::try_from(*count).unwrap_or(u64::MAX));
    }

    let mut sink = 0usize;
    let start = Instant::now();
    for _ in 0..iters {
        sink ^= black_box(mi.duplicated(DuplicateKeep::First)).len();
    }
    let duplicated_elapsed = start.elapsed();

    let start = Instant::now();
    for _ in 0..iters {
        sink ^= black_box(mi.nunique());
    }
    let nunique_elapsed = start.elapsed();

    let start = Instant::now();
    for _ in 0..iters {
        sink ^= black_box(mi.unique().len());
    }
    let unique_len_elapsed = start.elapsed();

    let mut reference_ns = Vec::with_capacity(iters);
    let mut value_counts_ns = Vec::with_capacity(iters);
    for iteration in 0..iters {
        if iteration % 2 == 0 {
            let start = Instant::now();
            sink ^= black_box(generic_value_counts(&mi)).len();
            reference_ns.push(start.elapsed().as_nanos());
            let start = Instant::now();
            sink ^= black_box(mi.value_counts()).len();
            value_counts_ns.push(start.elapsed().as_nanos());
        } else {
            let start = Instant::now();
            sink ^= black_box(mi.value_counts()).len();
            value_counts_ns.push(start.elapsed().as_nanos());
            let start = Instant::now();
            sink ^= black_box(generic_value_counts(&mi)).len();
            reference_ns.push(start.elapsed().as_nanos());
        }
    }
    let reference_p50_ns = percentile_ns(&mut reference_ns, 1, 2);
    let reference_p95_ns = percentile_ns(&mut reference_ns, 19, 20);
    let value_counts_p50_ns = percentile_ns(&mut value_counts_ns, 1, 2);
    let value_counts_p95_ns = percentile_ns(&mut value_counts_ns, 19, 20);

    println!(
        "mi_dedup n={n} iters={iters}: duplicated_ms={:.3} nunique_ms={:.3} unique_len_ms={:.3} value_counts_reference_p50_ns={reference_p50_ns} value_counts_reference_p95_ns={reference_p95_ns} value_counts_p50_ns={value_counts_p50_ns} value_counts_p95_ns={value_counts_p95_ns} kept={kept} nunique={nunique} value_counts_len={} chk={chk:016x} sink={sink}",
        duplicated_elapsed.as_secs_f64() * 1000.0 / iters as f64,
        nunique_elapsed.as_secs_f64() * 1000.0 / iters as f64,
        unique_len_elapsed.as_secs_f64() * 1000.0 / iters as f64,
        candidate_counts.len()
    );
}
