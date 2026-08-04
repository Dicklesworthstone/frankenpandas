//! Head-to-head-friendly RangeIndex set-op microbench.
//!
//! Run:
//!   cargo run -p fp-index --example bench_range_setops --release -- 1000000 200 overlap
//!   cargo run -p fp-index --example bench_range_setops --release -- 1000000 200 searchsorted
//!   cargo run -p fp-index --example bench_range_setops --release -- 1000000 9 affine_index_searchsorted
//!   cargo run -p fp-index --example bench_range_setops --release -- 1000000 50 putmask_where
//!   cargo run -p fp-index --example bench_range_setops --release -- 1000000 50 index_append_repeat
//!   cargo run -p fp-index --example bench_range_setops --release -- 1000000 50 index_drop_labels
//!   cargo run -p fp-index --example bench_range_setops --release -- 1000000 50 index_get_indexer_non_unique
//!   cargo run -p fp-index --example bench_range_setops --release -- 1000000 50 index_nunique
//!   cargo run -p fp-index --example bench_range_setops --release -- 1000000 50 index_diff
//!   cargo run -p fp-index --example bench_range_setops --release -- 1000000 50 index_position_lookup
//!   cargo run -p fp-index --example bench_range_setops --release -- 1000000 50 index_asof_locs
//!   cargo run -p fp-index --example bench_range_setops --release -- 1000000 50 index_sorted_setops
//!   cargo run -p fp-index --example bench_range_setops --release -- 1000000 50 index_hash_setops
//!   cargo run -p fp-index --example bench_range_setops --release -- 1000000 50 index_list_aliases
//!   cargo run -p fp-index --example bench_range_setops --release -- 1000000 200 index_from_range
//!   cargo run -p fp-index --example bench_range_setops --release -- 1000000 200 range_to_flat_index
//!   cargo run -p fp-index --example bench_range_setops --release -- 1000000 100 range_reindex
//!   cargo run -p fp-index --example bench_range_setops --release -- 1000000 50 range_asof_locs
//!   cargo run -p fp-index --example bench_range_setops --release -- 1000000 50 range_astype
//!   cargo run -p fp-index --example bench_range_setops --release -- 1000000 20 range_take_repeat
//!   cargo run -p fp-index --example bench_range_setops --release -- 1000000 20 range_splice_outputs
//!   cargo run -p fp-index --example bench_range_setops --release -- 1000000 200 range_median 64
//!   cargo run -p fp-index --example bench_range_setops --release -- 1000000 50 range_diff
//!   cargo run -p fp-index --example bench_range_setops --release -- 1000000 50 range_join
//!   cargo run -p fp-index --example bench_range_setops --release -- 1000000 50 range_isin
//!   cargo run -p fp-index --example bench_range_setops --release -- 1000000 25 date_range
//!   cargo run -p fp-index --example bench_range_setops --release -- 1000000 25 date_range_infer_freq
//!   cargo run -p fp-index --profile release-perf --example bench_range_setops -- 1000000 15 date_range_monotonic

use std::{hint::black_box, time::Instant};

use fp_index::{
    Index, IndexLabel, RangeIndex, date_range, infer_freq, infer_freq_from_nanos,
};
use fp_types::Timedelta;
use rustc_hash::FxHashMap;

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

fn paired_percentiles_ns(
    iters: usize,
    mut reference: impl FnMut() -> usize,
    mut candidate: impl FnMut() -> usize,
) -> ((u128, u128, u128), (u128, u128, u128), usize) {
    let mut sink = 0usize;
    for _ in 0..3 {
        sink ^= black_box(reference());
        sink ^= black_box(candidate());
    }
    let mut reference_samples = Vec::with_capacity(iters);
    let mut candidate_samples = Vec::with_capacity(iters);
    for iteration in 0..iters {
        let mut measure = |f: &mut dyn FnMut() -> usize, samples: &mut Vec<u128>| {
            let started = Instant::now();
            sink ^= black_box(f());
            samples.push(started.elapsed().as_nanos());
        };
        if iteration % 2 == 0 {
            measure(&mut reference, &mut reference_samples);
            measure(&mut candidate, &mut candidate_samples);
        } else {
            measure(&mut candidate, &mut candidate_samples);
            measure(&mut reference, &mut reference_samples);
        }
    }
    reference_samples.sort_unstable();
    candidate_samples.sort_unstable();
    let percentiles = |samples: &[u128]| {
        let Some(&maximum) = samples.last() else {
            return (0, 0, 0);
        };
        let last = samples.len() - 1;
        (
            samples.get(last / 2).copied().unwrap_or(maximum),
            samples.get(last * 95 / 100).copied().unwrap_or(maximum),
            maximum,
        )
    };
    (
        percentiles(&reference_samples),
        percentiles(&candidate_samples),
        sink,
    )
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

fn alternating_extreme_i64_values(start: i64, len: usize) -> Vec<i64> {
    (0..len)
        .map(|offset| {
            let rank = if offset % 2 == 0 {
                len - 1 - (offset / 2)
            } else {
                offset / 2
            };
            start + i64::try_from(rank).expect("rank fits i64")
        })
        .collect()
}

fn descending_i64_values(len: usize) -> Vec<i64> {
    (0..len)
        .rev()
        .map(|offset| i64::try_from(offset).expect("offset fits i64"))
        .collect()
}

fn lookup_probe_values(len: usize) -> Vec<i64> {
    let len = len.max(1);
    (0usize..4096)
        .map(|offset| {
            let value = offset.wrapping_mul(15_485_863) % len;
            i64::try_from(value).expect("probe label fits i64")
        })
        .collect()
}

fn take_positions(len: usize) -> Vec<usize> {
    let len = len.max(1);
    (0..len)
        .map(|offset| offset.wrapping_mul(15_485_863) % len)
        .collect()
}

fn asof_where_values(len: usize) -> Vec<i64> {
    let len_i64 = i64::try_from(len).expect("length fits i64");
    (0..len)
        .map(|offset| {
            let offset = i64::try_from(offset).expect("offset fits i64");
            offset
                .checked_mul(2)
                .and_then(|value| value.checked_sub((offset % 3) + 1))
                .unwrap_or(len_i64)
        })
        .collect()
}

fn stepped_i64_values(len: usize) -> Vec<i64> {
    (0..len)
        .map(|offset| {
            let offset = i64::try_from(offset).expect("offset fits i64");
            offset
                .checked_mul(3)
                .expect("benchmark value fits i64")
                .checked_add(offset.rem_euclid(7))
                .expect("benchmark value fits i64")
        })
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

fn wide_repeated_i64_values(len: usize, repeats: usize) -> Vec<i64> {
    const STRIDE: i64 = 1_000_003;
    (0..len)
        .map(|offset| {
            i64::try_from(offset / repeats)
                .expect("label fits i64")
                .checked_mul(STRIDE)
                .expect("wide label fits i64")
        })
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

fn range_isin_needles(len: usize, needle_count: usize) -> Vec<i64> {
    let len = len.max(1);
    let len_i64 = i64::try_from(len).expect("length fits i64");
    (0..needle_count)
        .map(|offset| {
            let rank = (offset / 2).wrapping_mul(15_485_863) % len;
            let hit = i64::try_from(rank).expect("needle label fits i64");
            match offset % 4 {
                0 | 1 => hit,
                2 => len_i64 + hit + 1,
                _ => -hit - 1,
            }
        })
        .collect()
}

fn bool_mask_digest(mask: &[bool]) -> usize {
    let mut digest = mask.len();
    for (position, bit) in [
        mask.first(),
        mask.get(mask.len() / 3),
        mask.get(mask.len() / 2),
        mask.get(mask.len().saturating_sub(1)),
    ]
    .into_iter()
    .flatten()
    .enumerate()
    {
        digest = digest
            .wrapping_mul(131)
            .wrapping_add(usize::from(*bit) << position);
    }
    digest
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

fn diff_digest(values: &[Option<IndexLabel>]) -> usize {
    let mut digest = values.len();
    for label in [values.first(), values.get(values.len() / 2), values.last()]
        .into_iter()
        .flatten()
        .flatten()
    {
        digest = index_label_digest(digest, label);
    }
    digest
}

fn fold_u64_digest(digest: usize, tag: usize, value: u64) -> usize {
    value
        .to_ne_bytes()
        .iter()
        .fold(digest.rotate_left(1).wrapping_add(tag), |acc, byte| {
            acc.wrapping_mul(131).wrapping_add(usize::from(*byte))
        })
}

fn index_label_digest(digest: usize, label: &IndexLabel) -> usize {
    match label {
        IndexLabel::Int64(value) => {
            fold_u64_digest(digest, 1, u64::from_ne_bytes(value.to_ne_bytes()))
        }
        IndexLabel::Utf8(value) => value
            .bytes()
            .fold(digest.rotate_left(1).wrapping_add(2), |acc, byte| {
                acc.wrapping_mul(131).wrapping_add(usize::from(byte))
            }),
        IndexLabel::Timedelta64(value) => {
            fold_u64_digest(digest, 3, u64::from_ne_bytes(value.to_ne_bytes()))
        }
        IndexLabel::Datetime64(value) => {
            fold_u64_digest(digest, 4, u64::from_ne_bytes(value.to_ne_bytes()))
        }
        IndexLabel::Float64(value) => fold_u64_digest(digest, 5, value.0.to_bits()),
        IndexLabel::Bool(value) => digest
            .rotate_left(1)
            .wrapping_mul(131)
            .wrapping_add(6 + usize::from(*value)),
        IndexLabel::Null(_) => digest.rotate_left(1).wrapping_mul(131).wrapping_add(8),
    }
}

fn label_vec_digest(values: &[IndexLabel]) -> usize {
    let mut digest = values.len();
    for label in [values.first(), values.get(values.len() / 2), values.last()]
        .into_iter()
        .flatten()
    {
        digest = index_label_digest(digest, label);
    }
    digest
}

fn index_shape_digest(index: &Index) -> usize {
    let mut digest = index.len();
    if let Some(name) = index.name() {
        for byte in name.bytes() {
            digest = digest.wrapping_mul(131).wrapping_add(usize::from(byte));
        }
    }
    digest
}

fn i64_option_digest(values: &[Option<i64>]) -> usize {
    let mut digest = values.len();
    for value in [values.first(), values.get(values.len() / 2), values.last()]
        .into_iter()
        .flatten()
    {
        let bits = value.unwrap_or(i64::MIN).to_ne_bytes();
        digest = bits.iter().fold(digest.rotate_left(1), |acc, byte| {
            acc.wrapping_mul(131).wrapping_add(usize::from(*byte))
        });
    }
    digest
}

fn scalar_lookup_digest(index: &Index, probes: &[i64]) -> usize {
    let mut digest = probes.len();
    for &probe in probes {
        let label = IndexLabel::Int64(probe);
        digest ^= index.position(&label).unwrap_or(usize::MAX).rotate_left(1);
        digest ^= index.get_loc(&label).unwrap_or(usize::MAX).rotate_left(3);
        digest = digest
            .wrapping_mul(131)
            .wrapping_add(usize::from(index.contains(&label)));
    }
    digest
}

fn option_position_digest(values: &[Option<usize>]) -> usize {
    let mut digest = values.len();
    for value in [values.first(), values.get(values.len() / 2), values.last()]
        .into_iter()
        .flatten()
    {
        digest =
            digest.wrapping_mul(131).rotate_left(1) ^ value.unwrap_or(usize::MAX).rotate_left(3);
    }
    digest
}

fn from_range_lookup_digest(start: i64, stop: i64, step: i64, target: i64) -> usize {
    let index = black_box(Index::from_range(
        black_box(start),
        black_box(stop),
        black_box(step),
    ));
    index.len()
        ^ index
            .get_loc(&IndexLabel::Int64(black_box(target)))
            .unwrap_or(usize::MAX)
            .rotate_left(1)
}

fn range_to_flat_lookup_digest(index: &RangeIndex, target: i64) -> usize {
    let flat = black_box(index).to_flat_index();
    flat.len()
        ^ flat
            .get_loc(&IndexLabel::Int64(black_box(target)))
            .unwrap_or(usize::MAX)
            .rotate_left(1)
}

fn range_reindex_digest(source: &RangeIndex, target: &RangeIndex) -> usize {
    let (reindexed, indexer) = black_box(source).reindex(black_box(target));
    reindexed.len() ^ indexer_digest(&indexer, &[])
}

fn range_reindex_legacy_target_values_digest(source: &RangeIndex, target: &RangeIndex) -> usize {
    let target_values = black_box(target).values();
    let indexer = black_box(source).get_indexer(black_box(&target_values));
    target.len() ^ indexer_digest(&indexer, &[])
}

fn range_asof_locs_digest(source: &RangeIndex, where_index: &Index) -> usize {
    let positions = black_box(source).asof_locs(black_box(where_index), None);
    option_position_digest(&positions)
}

fn label_materializing_nunique_digest(index: &Index) -> usize {
    let labels = black_box(index).values();
    let mut seen = FxHashMap::<&IndexLabel, ()>::default();
    for label in &labels {
        seen.insert(label, ());
    }
    seen.len()
}

fn range_median_digest(index: &RangeIndex, calls: usize) -> usize {
    let mut digest = calls;
    for _ in 0..calls {
        let bits = black_box(index)
            .median()
            .expect("benchmark range is non-empty")
            .to_bits();
        digest = digest
            .wrapping_mul(131)
            .wrapping_add(usize::try_from(bits & 0xffff_ffff).expect("lower bits fit usize"))
            .rotate_left(1);
    }
    digest
}

fn frequency_digest(frequency: Option<String>) -> usize {
    frequency.map_or(0, |value| {
        value.bytes().fold(0usize, |digest, byte| {
            digest.wrapping_mul(131).wrapping_add(usize::from(byte))
        })
    })
}

fn former_infer_freq_digest(index: &Index) -> usize {
    let mut values = Vec::with_capacity(index.len());
    for label in black_box(index).labels() {
        match label {
            IndexLabel::Datetime64(value) => match *value {
                i64::MIN => return 0,
                value => values.push(value),
            },
            _ => return usize::MAX,
        }
    }
    frequency_digest(infer_freq_from_nanos(&values).expect("valid benchmark frequency"))
}

fn public_infer_freq_digest(index: &Index) -> usize {
    frequency_digest(infer_freq(black_box(index)).expect("valid benchmark frequency"))
}

fn former_monotonic_digest(index: &Index) -> usize {
    let labels = black_box(index).labels();
    let increasing = labels.windows(2).all(|pair| pair[0] <= pair[1]);
    let decreasing = labels.windows(2).all(|pair| pair[0] >= pair[1]);
    usize::from(increasing) | (usize::from(decreasing) << 1)
}

fn public_monotonic_digest(index: &Index) -> usize {
    usize::from(black_box(index).is_monotonic_increasing())
        | (usize::from(black_box(index).is_monotonic_decreasing()) << 1)
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
    let extra: usize = args
        .get(4)
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    if scenario == "date_range_monotonic" {
        let make_index = || {
            date_range(
                Some("1970-01-01"),
                None,
                Some(n),
                Timedelta::NANOS_PER_SEC,
                Some("timestamp"),
            )
            .expect("valid benchmark date range")
        };
        let reference_index = make_index();
        let candidate_index = make_index();
        assert_eq!(
            former_monotonic_digest(&reference_index),
            public_monotonic_digest(&candidate_index),
            "public monotonic predicates must match the former label-window body",
        );
        let (control_a_ns, control_b_ns, control_sink) = paired_percentiles_ns(
            iters,
            || former_monotonic_digest(&reference_index),
            || former_monotonic_digest(&reference_index),
        );
        let (reference_ns, candidate_ns, candidate_sink) = paired_percentiles_ns(
            iters,
            || former_monotonic_digest(&reference_index),
            || public_monotonic_digest(&candidate_index),
        );
        let sink = control_sink ^ candidate_sink;
        println!(
            "date_range_monotonic n={n} control_a_p50_ns={} control_a_p95_ns={} control_a_max_ns={} control_b_p50_ns={} control_b_p95_ns={} control_b_max_ns={} reference_p50_ns={} reference_p95_ns={} reference_max_ns={} candidate_p50_ns={} candidate_p95_ns={} candidate_max_ns={} sink={sink}",
            control_a_ns.0,
            control_a_ns.1,
            control_a_ns.2,
            control_b_ns.0,
            control_b_ns.1,
            control_b_ns.2,
            reference_ns.0,
            reference_ns.1,
            reference_ns.2,
            candidate_ns.0,
            candidate_ns.1,
            candidate_ns.2,
        );
        return;
    }
    if scenario == "date_range" {
        let reference = || {
            let nanos = (0..n)
                .map(|offset| {
                    i64::try_from(offset)
                        .expect("offset fits i64")
                        .checked_mul(Timedelta::NANOS_PER_SEC)
                        .expect("benchmark timestamp fits i64")
                })
                .collect();
            let index = Index::from_datetime64(nanos).set_name("timestamp");
            black_box(&index);
            index.len() ^ index.name().map_or(0, str::len)
        };
        let candidate = || {
            let index = date_range(
                Some("1970-01-01"),
                None,
                Some(n),
                Timedelta::NANOS_PER_SEC,
                Some("timestamp"),
            )
            .expect("valid benchmark date range");
            black_box(&index);
            index.len() ^ index.name().map_or(0, str::len)
        };
        let (reference_ns, candidate_ns, sink) = paired_percentiles_ns(iters, reference, candidate);
        println!(
            "date_range n={n} reference_p50_ns={} reference_p95_ns={} reference_p99_ns={} candidate_p50_ns={} candidate_p95_ns={} candidate_p99_ns={} sink={sink}",
            reference_ns.0,
            reference_ns.1,
            reference_ns.2,
            candidate_ns.0,
            candidate_ns.1,
            candidate_ns.2,
        );
        return;
    }
    if scenario == "date_range_infer_freq" {
        let index = date_range(
            Some("1970-01-01"),
            None,
            Some(n),
            Timedelta::NANOS_PER_SEC,
            Some("timestamp"),
        )
        .expect("valid benchmark date range");
        assert_eq!(
            former_infer_freq_digest(&index),
            public_infer_freq_digest(&index),
            "public frequency inference must match the former materializing body",
        );
        let (control_a_ns, control_b_ns, control_sink) = paired_percentiles_ns(
            iters,
            || former_infer_freq_digest(&index),
            || former_infer_freq_digest(&index),
        );
        let (reference_ns, candidate_ns, candidate_sink) = paired_percentiles_ns(
            iters,
            || former_infer_freq_digest(&index),
            || public_infer_freq_digest(&index),
        );
        let sink = control_sink ^ candidate_sink;
        println!(
            "date_range_infer_freq n={n} control_a_p50_ns={} control_a_p95_ns={} control_a_max_ns={} control_b_p50_ns={} control_b_p95_ns={} control_b_max_ns={} reference_p50_ns={} reference_p95_ns={} reference_max_ns={} candidate_p50_ns={} candidate_p95_ns={} candidate_max_ns={} sink={sink}",
            control_a_ns.0,
            control_a_ns.1,
            control_a_ns.2,
            control_b_ns.0,
            control_b_ns.1,
            control_b_ns.2,
            reference_ns.0,
            reference_ns.1,
            reference_ns.2,
            candidate_ns.0,
            candidate_ns.1,
            candidate_ns.2,
        );
        return;
    }
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
    if scenario == "affine_index_searchsorted" {
        let probes = searchsorted_probes(n);
        let probe_count = probes.len();
        let (searchsorted_ns, sink) = best_ns(iters, || {
            let index = Index::new_known_unique_int64_affine_range(0, 2, n)
                .expect("valid affine search index");
            let mut acc = 0usize;
            for &probe in &probes {
                let label = IndexLabel::Int64(probe);
                let left = index
                    .searchsorted(&label, "left")
                    .expect("left searchsorted");
                let right = index
                    .searchsorted(&label, "right")
                    .expect("right searchsorted");
                acc = acc.wrapping_add(left).rotate_left(1) ^ right;
            }
            acc
        });
        println!(
            "affine_index_searchsorted n={n} probes={probe_count} searchsorted_ns={searchsorted_ns} sink={sink}"
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
    if scenario == "range_take_repeat" {
        let n_i64 = i64::try_from(n).expect("n fits i64");
        let index = RangeIndex::new(0, n_i64 * 3, 3)
            .expect("valid take range")
            .set_name("row");
        let positions = take_positions(n);
        let position_count = positions.len();
        let (take_ns, take_sink) = best_ns(iters, || {
            let output = index.take(&positions).expect("take");
            int64_index_digest(&output)
        });
        let (repeat_ns, repeat_sink) = best_ns(iters, || {
            let output = index.repeat(2);
            int64_index_digest(&output)
        });
        let sink = take_sink ^ repeat_sink;
        println!(
            "range_take_repeat n={n} positions={position_count} take_ns={take_ns} repeat2_ns={repeat_ns} sink={sink}"
        );
        return;
    }
    if scenario == "range_splice_outputs" {
        let n_i64 = i64::try_from(n).expect("n fits i64");
        let left = RangeIndex::new(0, n_i64 * 3, 3)
            .expect("valid left range")
            .set_name("row");
        let right = RangeIndex::new(n_i64 * 5, n_i64 * 7, 2)
            .expect("valid right range")
            .set_name("row");
        let insert_loc = n / 2;
        let delete_loc = n / 2;
        let (insert_ns, insert_sink) = best_ns(iters, || {
            let output = left.insert(insert_loc, -7).expect("insert");
            black_box(output).len()
        });
        let (append_ns, append_sink) = best_ns(iters, || {
            let output = left.append(&right);
            black_box(output).len()
        });
        let (delete_ns, delete_sink) = best_ns(iters, || {
            let output = left.delete(delete_loc).expect("delete");
            black_box(output).len()
        });
        let sink = insert_sink ^ append_sink ^ delete_sink;
        println!(
            "range_splice_outputs n={n} insert_loc={insert_loc} delete_loc={delete_loc} insert_ns={insert_ns} append_ns={append_ns} delete_ns={delete_ns} sink={sink}"
        );
        return;
    }
    if scenario == "range_median" {
        let n_i64 = i64::try_from(n).expect("n fits i64");
        let calls = extra.max(1);
        let unit = RangeIndex::new(0, n_i64, 1).expect("valid unit range");
        let strided = RangeIndex::new(
            0,
            n_i64.checked_mul(3).expect("benchmark range stop fits i64"),
            3,
        )
        .expect("valid strided range");
        let descending = RangeIndex::new(n_i64, 0, -1).expect("valid descending range");
        let (unit_ns, unit_sink) = best_ns(iters, || range_median_digest(&unit, calls));
        let (strided_ns, strided_sink) = best_ns(iters, || range_median_digest(&strided, calls));
        let (descending_ns, descending_sink) =
            best_ns(iters, || range_median_digest(&descending, calls));
        let sink = unit_sink ^ strided_sink ^ descending_sink;
        println!(
            "range_median n={n} calls={calls} unit_ns={unit_ns} strided_ns={strided_ns} descending_ns={descending_ns} sink={sink}"
        );
        return;
    }
    if scenario == "range_diff" {
        let n_i64 = i64::try_from(n).expect("n fits i64");
        let unit = RangeIndex::new(0, n_i64, 1).expect("valid unit range");
        let strided = RangeIndex::new(
            0,
            n_i64.checked_mul(3).expect("benchmark range stop fits i64"),
            3,
        )
        .expect("valid strided range");
        let descending = RangeIndex::new(n_i64, 0, -1).expect("valid descending range");
        let periods = 1i64;
        let (unit_ns, unit_sink) = best_ns(iters, || i64_option_digest(&unit.diff(periods)));
        let (strided_ns, strided_sink) =
            best_ns(iters, || i64_option_digest(&strided.diff(periods)));
        let (descending_ns, descending_sink) =
            best_ns(iters, || i64_option_digest(&descending.diff(periods)));
        let sink = unit_sink ^ strided_sink ^ descending_sink;
        println!(
            "range_diff n={n} periods={periods} unit_ns={unit_ns} strided_ns={strided_ns} descending_ns={descending_ns} sink={sink}"
        );
        return;
    }
    if scenario == "range_join" {
        let n_i64 = i64::try_from(n).expect("n fits i64");
        let range = RangeIndex::new(0, n_i64, 1)
            .expect("valid join range")
            .set_name("k");
        let other = Index::from_i64_values(sequential_i64_values(n_i64 / 2, n)).set_name("k");
        let (inner_ns, inner_sink) = best_ns(iters, || {
            let output = range.join(&other, "inner").expect("inner join");
            int64_index_digest(&output)
        });
        let (outer_ns, outer_sink) = best_ns(iters, || {
            let output = range.join(&other, "outer").expect("outer join");
            int64_index_digest(&output)
        });
        let sink = inner_sink ^ outer_sink;
        println!("range_join n={n} inner_ns={inner_ns} outer_ns={outer_ns} sink={sink}");
        return;
    }
    if scenario == "range_isin" {
        let n_i64 = i64::try_from(n).expect("n fits i64");
        let range = RangeIndex::new(0, n_i64, 1).expect("valid isin range");
        let small_needles = range_isin_needles(n, 1024);
        let large_needles = range_isin_needles(n, n / 2);
        let (small_ns, small_sink) =
            best_ns(iters, || bool_mask_digest(&range.isin(&small_needles)));
        let (large_ns, large_sink) =
            best_ns(iters, || bool_mask_digest(&range.isin(&large_needles)));
        let sink = small_sink ^ large_sink;
        println!(
            "range_isin n={n} small_needles={} large_needles={} small_ns={small_ns} large_ns={large_ns} sink={sink}",
            small_needles.len(),
            large_needles.len()
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
    if scenario == "index_nunique" {
        let repeats = 4usize;
        let unique_len = n.div_ceil(repeats);
        let dense = Index::from_i64_values(repeated_i64_values(n, repeats)).set_name("row");
        let wide = Index::from_i64_values(wide_repeated_i64_values(n, repeats)).set_name("row");
        let (dense_ns, dense_sink) = best_ns(iters, || black_box(&dense).nunique());
        let (dense_label_ns, dense_label_sink) =
            best_ns(iters, || label_materializing_nunique_digest(&dense));
        let (wide_ns, wide_sink) = best_ns(iters, || black_box(&wide).nunique());
        let (wide_label_ns, wide_label_sink) =
            best_ns(iters, || label_materializing_nunique_digest(&wide));
        let sink = dense_sink ^ dense_label_sink ^ wide_sink ^ wide_label_sink;
        println!(
            "index_nunique n={n} repeats={repeats} unique_len={unique_len} dense_ns={dense_ns} dense_label_materializing_ns={dense_label_ns} wide_ns={wide_ns} wide_label_materializing_ns={wide_label_ns} sink={sink}"
        );
        return;
    }
    if scenario == "index_diff" {
        let index = Index::from_i64_values(stepped_i64_values(n)).set_name("row");
        let periods = 1usize;
        let (diff_ns, sink) = best_ns(iters, || {
            let output = index.diff(periods);
            diff_digest(&output)
        });
        println!("index_diff n={n} periods={periods} diff_ns={diff_ns} sink={sink}");
        return;
    }
    if scenario == "index_position_lookup" {
        let sorted = Index::from_i64_values(sequential_i64_values(0, n)).set_name("row");
        let unsorted = Index::from_i64_values(descending_i64_values(n)).set_name("row");
        let probes = lookup_probe_values(n);
        let probe_count = probes.len();
        let (sorted_ns, sorted_sink) = best_ns(iters, || scalar_lookup_digest(&sorted, &probes));
        let (unsorted_ns, unsorted_sink) =
            best_ns(iters, || scalar_lookup_digest(&unsorted, &probes));
        let sink = sorted_sink ^ unsorted_sink;
        println!(
            "index_position_lookup n={n} probes={probe_count} sorted_ns={sorted_ns} unsorted_ns={unsorted_ns} sink={sink}"
        );
        return;
    }
    if scenario == "index_asof_locs" {
        let source = Index::from_i64_values(
            (0..n)
                .map(|offset| {
                    i64::try_from(offset)
                        .expect("offset fits i64")
                        .checked_mul(2)
                        .expect("source label fits i64")
                })
                .collect(),
        );
        let where_index = Index::from_i64_values(asof_where_values(n));
        let where_len = where_index.len();
        let (asof_locs_ns, sink) = best_ns(iters, || {
            let output = source.asof_locs(&where_index, None);
            option_position_digest(&output)
        });
        println!(
            "index_asof_locs n={n} where_len={where_len} asof_locs_ns={asof_locs_ns} sink={sink}"
        );
        return;
    }
    if scenario == "index_sorted_setops" {
        let n_i64 = i64::try_from(n).expect("n fits i64");
        let left = Index::from_i64_values(sequential_i64_values(0, n)).set_name("row");
        let right = Index::from_i64_values(sequential_i64_values(n_i64 / 2, n));
        let (intersection_ns, intersection_sink) = best_ns(iters, || {
            let output = left.intersection(&right);
            int64_index_digest(&output)
        });
        let (difference_ns, difference_sink) = best_ns(iters, || {
            let output = left.difference(&right);
            int64_index_digest(&output)
        });
        let sink = intersection_sink ^ difference_sink;
        println!(
            "index_sorted_setops n={n} intersection_ns={intersection_ns} difference_ns={difference_ns} sink={sink}"
        );
        return;
    }
    if scenario == "index_hash_setops" {
        let n_i64 = i64::try_from(n).expect("n fits i64");
        let left = Index::from_i64_values(alternating_extreme_i64_values(0, n)).set_name("row");
        let right = Index::from_i64_values(alternating_extreme_i64_values(n_i64 / 2, n));
        let (intersection_ns, intersection_sink) = best_ns(iters, || {
            let output = left.intersection(&right);
            int64_index_digest(&output)
        });
        let (union_ns, union_sink) = best_ns(iters, || {
            let output = left.union_with(&right);
            int64_index_digest(&output)
        });
        let (difference_ns, difference_sink) = best_ns(iters, || {
            let output = left.difference(&right);
            int64_index_digest(&output)
        });
        let (symmetric_difference_ns, symmetric_difference_sink) = best_ns(iters, || {
            let output = left.symmetric_difference(&right);
            int64_index_digest(&output)
        });
        let sink = intersection_sink ^ union_sink ^ difference_sink ^ symmetric_difference_sink;
        println!(
            "index_hash_setops n={n} intersection_ns={intersection_ns} union_ns={union_ns} difference_ns={difference_ns} symmetric_difference_ns={symmetric_difference_ns} sink={sink}"
        );
        return;
    }
    if scenario == "index_list_aliases" {
        let n_i64 = i64::try_from(n).expect("n fits i64");
        let index = Index::from_range(
            0,
            n_i64.checked_mul(3).expect("benchmark range stop fits i64"),
            3,
        )
        .set_name("row");
        let (to_list_ns, to_list_sink) = best_ns(iters, || label_vec_digest(&index.to_list()));
        let (values_ns, values_sink) = best_ns(iters, || label_vec_digest(&index.values()));
        let (ravel_ns, ravel_sink) = best_ns(iters, || label_vec_digest(&index.ravel()));
        let sink = to_list_sink ^ values_sink ^ ravel_sink;
        println!(
            "index_list_aliases n={n} to_list_ns={to_list_ns} values_ns={values_ns} ravel_ns={ravel_ns} sink={sink}"
        );
        return;
    }
    if scenario == "index_from_range" {
        let n_i64 = i64::try_from(n).expect("n fits i64");
        let (unit_ns, unit_sink) =
            best_ns(iters, || from_range_lookup_digest(0, n_i64, 1, n_i64 / 2));
        let (strided_ns, strided_sink) = best_ns(iters, || {
            from_range_lookup_digest(
                0,
                n_i64.checked_mul(3).expect("benchmark range stop fits i64"),
                3,
                (n_i64 / 2)
                    .checked_mul(3)
                    .expect("benchmark range target fits i64"),
            )
        });
        let (descending_ns, descending_sink) =
            best_ns(iters, || from_range_lookup_digest(n_i64, 0, -1, n_i64 / 2));
        let sink = unit_sink ^ strided_sink ^ descending_sink;
        println!(
            "index_from_range n={n} unit_ns={unit_ns} strided_ns={strided_ns} descending_ns={descending_ns} sink={sink}"
        );
        return;
    }
    if scenario == "range_to_flat_index" {
        let n_i64 = i64::try_from(n).expect("n fits i64");
        let unit = RangeIndex::new(0, n_i64, 1)
            .expect("valid unit range")
            .set_name("row");
        let strided = RangeIndex::new(
            0,
            n_i64.checked_mul(3).expect("benchmark range stop fits i64"),
            3,
        )
        .expect("valid strided range")
        .set_name("row");
        let descending = RangeIndex::new(n_i64, 0, -1)
            .expect("valid descending range")
            .set_name("row");
        let (unit_ns, unit_sink) = best_ns(iters, || range_to_flat_lookup_digest(&unit, n_i64 / 2));
        let (strided_ns, strided_sink) = best_ns(iters, || {
            range_to_flat_lookup_digest(
                &strided,
                (n_i64 / 2)
                    .checked_mul(3)
                    .expect("benchmark range target fits i64"),
            )
        });
        let (descending_ns, descending_sink) = best_ns(iters, || {
            range_to_flat_lookup_digest(&descending, n_i64 / 2)
        });
        let sink = unit_sink ^ strided_sink ^ descending_sink;
        println!(
            "range_to_flat_index n={n} unit_ns={unit_ns} strided_ns={strided_ns} descending_ns={descending_ns} sink={sink}"
        );
        return;
    }
    if scenario == "range_reindex" {
        let n_i64 = i64::try_from(n).expect("n fits i64");
        let span = n_i64.checked_mul(3).expect("benchmark range span fits i64");
        let half_offset = (n_i64 / 2)
            .checked_mul(3)
            .expect("benchmark half offset fits i64");
        let target_stop = half_offset
            .checked_add(span)
            .expect("benchmark target stop fits i64");
        let source = RangeIndex::new(0, span, 3)
            .expect("valid reindex source")
            .set_name("row");
        let target = RangeIndex::new(half_offset, target_stop, 3)
            .expect("valid reindex target")
            .set_name("target");
        let descending_start = span
            .checked_sub(half_offset)
            .expect("benchmark descending start fits i64");
        let descending_stop = descending_start
            .checked_sub(span)
            .expect("benchmark descending stop fits i64");
        let descending_source = RangeIndex::new(span, 0, -3)
            .expect("valid descending reindex source")
            .set_name("row");
        let descending_target = RangeIndex::new(descending_start, descending_stop, -3)
            .expect("valid descending reindex target")
            .set_name("target");
        let (current_ns, current_sink) = best_ns(iters, || range_reindex_digest(&source, &target));
        let (legacy_target_values_ns, legacy_sink) = best_ns(iters, || {
            range_reindex_legacy_target_values_digest(&source, &target)
        });
        let (descending_ns, descending_sink) = best_ns(iters, || {
            range_reindex_digest(&descending_source, &descending_target)
        });
        let (descending_legacy_target_values_ns, descending_legacy_sink) = best_ns(iters, || {
            range_reindex_legacy_target_values_digest(&descending_source, &descending_target)
        });
        let sink = current_sink ^ legacy_sink ^ descending_sink ^ descending_legacy_sink;
        println!(
            "range_reindex n={n} current_ns={current_ns} legacy_target_values_ns={legacy_target_values_ns} descending_ns={descending_ns} descending_legacy_target_values_ns={descending_legacy_target_values_ns} sink={sink}"
        );
        return;
    }
    if scenario == "range_asof_locs" {
        let n_i64 = i64::try_from(n).expect("n fits i64");
        let source = RangeIndex::new(
            0,
            n_i64.checked_mul(2).expect("benchmark range stop fits i64"),
            2,
        )
        .expect("valid asof_locs source")
        .set_name("row");
        let where_index = Index::from_i64_values(asof_where_values(n));
        let where_len = where_index.len();
        let (asof_locs_ns, sink) = best_ns(iters, || range_asof_locs_digest(&source, &where_index));
        println!(
            "range_asof_locs n={n} where_len={where_len} asof_locs_ns={asof_locs_ns} sink={sink}"
        );
        return;
    }
    if scenario == "range_astype" {
        let n_i64 = i64::try_from(n).expect("n fits i64");
        let stop = n_i64.checked_mul(3).expect("benchmark range stop fits i64");
        let ascending = RangeIndex::new(0, stop, 3)
            .expect("valid ascending astype range")
            .set_name("row");
        let descending = RangeIndex::new(stop, 0, -3)
            .expect("valid descending astype range")
            .set_name("row");
        let (int64_ns, int64_sink) = best_ns(iters, || {
            let output = ascending.astype("int64").expect("astype int64");
            index_shape_digest(&output)
        });
        let (string_ns, string_sink) = best_ns(iters, || {
            let output = ascending.astype("string").expect("astype string");
            label_vec_digest(output.labels())
        });
        let (descending_string_ns, descending_string_sink) = best_ns(iters, || {
            let output = descending
                .astype("string")
                .expect("descending astype string");
            label_vec_digest(output.labels())
        });
        let sink = int64_sink ^ string_sink ^ descending_string_sink;
        println!(
            "range_astype n={n} int64_ns={int64_ns} string_ns={string_ns} descending_string_ns={descending_string_ns} sink={sink}"
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
