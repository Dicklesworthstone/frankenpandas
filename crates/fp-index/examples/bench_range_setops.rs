//! Head-to-head-friendly RangeIndex set-op microbench.
//!
//! Run:
//!   cargo run -p fp-index --example bench_range_setops --release -- 1000000 200 overlap
//!   cargo run -p fp-index --example bench_range_setops --release -- 1000000 200 searchsorted
//!   cargo run -p fp-index --example bench_range_setops --release -- 1000000 50 putmask_where
//!   cargo run -p fp-index --example bench_range_setops --release -- 1000000 50 index_append_repeat
//!   cargo run -p fp-index --example bench_range_setops --release -- 1000000 50 index_drop_labels
//!   cargo run -p fp-index --example bench_range_setops --release -- 1000000 50 index_get_indexer_non_unique

use std::{hint::black_box, time::Instant};

use fp_index::{Index, IndexLabel, RangeIndex};

fn best_ns(iters: usize, mut f: impl FnMut() -> usize) -> (u128, usize) {
    let mut sink = 0usize;
    for _ in 0..3 {
        sink ^= black_box(f());
    }
    let mut best = u128::MAX;
    for _ in 0..iters {
        let started = Instant::now();
        sink ^= black_box(f());
        let elapsed = started.elapsed().as_nanos();
        best = best.min(elapsed);
    }
    (best, sink)
}

fn ranges(n: usize, scenario: &str) -> (RangeIndex, RangeIndex) {
    let n = i64::try_from(n).expect("n fits i64");
    match scenario {
        "adjacent" => (
            RangeIndex::new(0, n, 1).expect("valid left range"),
            RangeIndex::new(n, n * 2, 1).expect("valid right range"),
        ),
        "descending" => (
            RangeIndex::new(n, 0, -1).expect("valid left range"),
            RangeIndex::new(n / 2, -(n / 2), -1).expect("valid right range"),
        ),
        _ => (
            RangeIndex::new(0, n, 1).expect("valid left range"),
            RangeIndex::new(n / 2, n + n / 2, 1).expect("valid right range"),
        ),
    }
}

fn searchsorted_probes(n: usize) -> Vec<i64> {
    let span = n.saturating_mul(2).saturating_add(1_000);
    (0usize..4096)
        .map(|i| {
            let value = i.wrapping_mul(15_485_863) % span;
            i64::try_from(value).expect("probe fits i64") - 500
        })
        .collect()
}

fn alternating_mask(n: usize) -> Vec<bool> {
    (0..n).map(|i| i % 2 == 1).collect()
}

fn int64_index_digest(index: &fp_index::Index) -> usize {
    let values = index
        .int64_label_values()
        .expect("benchmark output keeps typed Int64 backing");
    let mut digest = values.len();
    for value in [values.first(), values.get(values.len() / 2), values.last()]
        .into_iter()
        .flatten()
    {
        digest = value
            .to_ne_bytes()
            .iter()
            .fold(digest.rotate_left(1), |acc, byte| {
                acc.wrapping_mul(131).wrapping_add(usize::from(*byte))
            });
    }
    digest
}

fn sequential_i64_values(start: i64, len: usize) -> Vec<i64> {
    (0..len)
        .map(|offset| start + i64::try_from(offset).expect("offset fits i64"))
        .collect()
}

fn quarter_drop_labels(len: usize) -> Vec<IndexLabel> {
    (0..len)
        .step_by(4)
        .map(|offset| IndexLabel::Int64(i64::try_from(offset).expect("offset fits i64")))
        .collect()
}

fn repeated_i64_values(len: usize, repeats: usize) -> Vec<i64> {
    (0..len)
        .map(|offset| i64::try_from(offset / repeats).expect("label fits i64"))
        .collect()
}

fn non_unique_target_values(unique_len: usize) -> Vec<i64> {
    let unique_len_i64 = i64::try_from(unique_len).expect("unique length fits i64");
    (0..unique_len)
        .map(|offset| {
            let value = i64::try_from(offset).expect("target label fits i64");
            if offset % 4 == 3 {
                unique_len_i64 + value
            } else {
                value
            }
        })
        .collect()
}

fn indexer_digest(indexer: &[isize], missing: &[usize]) -> usize {
    let mut digest = indexer.len() ^ missing.len().rotate_left(1);
    for value in [
        indexer.first(),
        indexer.get(indexer.len() / 2),
        indexer.last(),
    ]
    .into_iter()
    .flatten()
    {
        digest = value
            .to_ne_bytes()
            .iter()
            .fold(digest.rotate_left(1), |acc, byte| {
                acc.wrapping_mul(131).wrapping_add(usize::from(*byte))
            });
    }
    for value in [
        missing.first(),
        missing.get(missing.len() / 2),
        missing.last(),
    ]
    .into_iter()
    .flatten()
    {
        digest = digest.wrapping_mul(131).wrapping_add(*value).rotate_left(1);
    }
    digest
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let n: usize = args
        .get(1)
        .and_then(|value| value.parse().ok())
        .unwrap_or(1_000_000);
    let iters: usize = args
        .get(2)
        .and_then(|value| value.parse().ok())
        .unwrap_or(200);
    let scenario = args.get(3).map_or("overlap", String::as_str);
    if scenario == "searchsorted" {
        let n_i64 = i64::try_from(n).expect("n fits i64");
        let index = RangeIndex::new(0, n_i64 * 2, 2).expect("valid search range");
        let probes = searchsorted_probes(n);
        let probe_count = probes.len();
        let (searchsorted_ns, sink) = best_ns(iters, || {
            let mut acc = 0usize;
            for &probe in &probes {
                let left = index
                    .searchsorted(probe, "left")
                    .expect("left searchsorted");
                let right = index
                    .searchsorted(probe, "right")
                    .expect("right searchsorted");
                acc = acc.wrapping_add(left).rotate_left(1) ^ right;
            }
            acc
        });
        println!(
            "range_searchsorted n={n} probes={probe_count} searchsorted_ns={searchsorted_ns} sink={sink}"
        );
        return;
    }
    if scenario == "putmask_where" {
        let n_i64 = i64::try_from(n).expect("n fits i64");
        let index = RangeIndex::new(0, n_i64 * 2, 2).expect("valid mask range");
        let mask = alternating_mask(n);
        let (putmask_ns, putmask_sink) = best_ns(iters, || {
            let output = index.putmask(&mask, -7).expect("putmask");
            int64_index_digest(&output)
        });
        let (where_ns, where_sink) = best_ns(iters, || {
            let output = index.r#where(&mask, -7).expect("where");
            int64_index_digest(&output)
        });
        let sink = putmask_sink ^ where_sink;
        println!(
            "range_putmask_where n={n} putmask_ns={putmask_ns} where_ns={where_ns} sink={sink}"
        );
        return;
    }
    if scenario == "index_append_repeat" {
        let n_i64 = i64::try_from(n).expect("n fits i64");
        let left = Index::from_i64_values(sequential_i64_values(0, n)).set_name("row");
        let right = Index::from_i64_values(sequential_i64_values(n_i64, n));
        let (append_ns, append_sink) = best_ns(iters, || {
            let output = left.append(&right);
            int64_index_digest(&output)
        });
        let (repeat_ns, repeat_sink) = best_ns(iters, || {
            let output = left.repeat(2);
            int64_index_digest(&output)
        });
        let sink = append_sink ^ repeat_sink;
        println!(
            "index_append_repeat n={n} append_ns={append_ns} repeat2_ns={repeat_ns} sink={sink}"
        );
        return;
    }
    if scenario == "index_drop_labels" {
        let index = Index::from_i64_values(sequential_i64_values(0, n)).set_name("row");
        let labels_to_drop = quarter_drop_labels(n);
        let drop_count = labels_to_drop.len();
        let (drop_ns, sink) = best_ns(iters, || {
            let output = index.drop_labels(&labels_to_drop);
            int64_index_digest(&output)
        });
        println!("index_drop_labels n={n} drop_count={drop_count} drop_ns={drop_ns} sink={sink}");
        return;
    }
    if scenario == "index_get_indexer_non_unique" {
        let repeats = 4usize;
        let unique_len = n / repeats;
        let source = Index::from_i64_values(repeated_i64_values(n, repeats)).set_name("row");
        let target_values = non_unique_target_values(unique_len);
        let target = Index::from_i64_values(target_values);
        let target_len = target.len();
        let missing_count = target_len / 4;
        let (non_unique_ns, sink) = best_ns(iters, || {
            let (indexer, missing) = source.get_indexer_non_unique(&target);
            indexer_digest(&indexer, &missing)
        });
        println!(
            "index_get_indexer_non_unique n={n} repeats={repeats} target_len={target_len} missing_count={missing_count} non_unique_ns={non_unique_ns} sink={sink}"
        );
        return;
    }

    let (left, right) = ranges(n, scenario);

    let (intersection_ns, intersection_sink) = best_ns(iters, || left.intersection(&right).len());
    let (union_ns, union_sink) = best_ns(iters, || left.union(&right).len());
    let (difference_ns, difference_sink) = best_ns(iters, || left.difference(&right).len());
    let (symmetric_difference_ns, symmetric_difference_sink) =
        best_ns(iters, || left.symmetric_difference(&right).len());
    let sink = intersection_sink ^ union_sink ^ difference_sink ^ symmetric_difference_sink;

    println!(
        "range_setops n={n} scenario={scenario} intersection_ns={intersection_ns} union_ns={union_ns} difference_ns={difference_ns} symmetric_difference_ns={symmetric_difference_ns} sink={sink}"
    );
}
