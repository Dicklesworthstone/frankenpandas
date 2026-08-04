#![cfg_attr(test, feature(test))]

//! Bench + golden harness for the dense-Int64 groupby median fast path
//! (`try_groupby_median_dense_int64`).
//!
//! Usage:
//!   cargo run --release --example bench_groupby_median -- bench   # timing
//!   cargo run --release --example bench_groupby_median -- golden  # digest stream
//!
//! `golden` prints every output (key, value-bits) pair in result order so an
//! external `sha256sum` can certify the result is bit-identical before/after a
//! change. `bench` reports the min wall-time over repeated runs.

use std::time::Instant;

use fp_columnar::Column;
use fp_frame::Series;
use fp_groupby::{AggFunc, GroupByOptions, groupby_agg, groupby_mean, groupby_min, groupby_sum};
use fp_index::{Index, IndexLabel};
use fp_runtime::{EvidenceLedger, RuntimePolicy};
use fp_types::Scalar;

#[cfg(test)]
extern crate test;

/// Deterministic splitmix64 so the harness needs no rand dependency and is
/// reproducible across before/after builds.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

fn build_data(n: usize, num_groups: i64) -> (Series, Series) {
    let mut rng = Rng(0x1234_5678_9ABC_DEF0);
    let mut idx = Vec::with_capacity(n);
    let mut keys = Vec::with_capacity(n);
    let mut vals = Vec::with_capacity(n);
    for i in 0..n {
        idx.push(IndexLabel::Int64(i as i64));
        let k = (rng.next() % num_groups as u64) as i64;
        keys.push(Scalar::Int64(k));
        // Spread of finite floats; a few integral values to exercise ties.
        let v = (rng.next() % 100_000) as f64 * 0.5;
        vals.push(Scalar::Float64(v));
    }
    let key_series = Series::from_values("key", idx.clone(), keys).expect("keys");
    let val_series = Series::from_values("value", idx, vals).expect("vals");
    (key_series, val_series)
}

fn build_typed_i64_sum_data(n: usize, num_groups: i64) -> (Series, Series) {
    let mut rng = Rng(0xD3A5_E1A7_51CE_0042);
    let mut keys = Vec::with_capacity(n);
    let mut vals = Vec::with_capacity(n);
    for _ in 0..n {
        keys.push((rng.next() % num_groups as u64) as i64);
        vals.push((rng.next() % 2_001) as i64 - 1_000);
    }
    let stop = i64::try_from(n).expect("benchmark length fits i64");
    let index = Index::from_range(0, stop, 1);
    let key_series =
        Series::new("key", index.clone(), Column::from_i64_values_owned(keys)).expect("typed keys");
    let val_series =
        Series::new("value", index, Column::from_i64_values_owned(vals)).expect("typed values");
    (key_series, val_series)
}

fn median_nanos(samples: &mut [u128]) -> u128 {
    samples.sort_unstable();
    samples[samples.len() / 2]
}

fn measure_nanos<T>(f: impl FnOnce() -> T) -> u128 {
    let start = Instant::now();
    std::hint::black_box(f());
    start.elapsed().as_nanos()
}

fn run_sum_attribution() {
    const N: usize = 250_000;
    const GROUPS: i64 = 1_000;
    const SAMPLES: usize = 11;
    let policy = RuntimePolicy::strict();
    let options = GroupByOptions::default();

    let (cold_keys, cold_vals) = build_typed_i64_sum_data(N, GROUPS);
    let mut cold_ledger = EvidenceLedger::new();
    let cold_out =
        groupby_sum(&cold_keys, &cold_vals, options, &policy, &mut cold_ledger).expect("cold sum");
    let (warm_keys, warm_vals) = build_typed_i64_sum_data(N, GROUPS);
    std::hint::black_box(warm_keys.values());
    std::hint::black_box(warm_vals.values());
    let mut warm_ledger = EvidenceLedger::new();
    let warm_out =
        groupby_sum(&warm_keys, &warm_vals, options, &policy, &mut warm_ledger).expect("warm sum");
    assert_eq!(cold_out.index(), warm_out.index());
    assert_eq!(cold_out.values(), warm_out.values());

    let mut materialize = Vec::with_capacity(SAMPLES);
    let mut warm_kernel = Vec::with_capacity(SAMPLES);
    let mut cold_public = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let (keys, vals) = build_typed_i64_sum_data(N, GROUPS);
        materialize.push(measure_nanos(|| {
            (
                std::hint::black_box(keys.values()).len(),
                std::hint::black_box(vals.values()).len(),
            )
        }));

        let (keys, vals) = build_typed_i64_sum_data(N, GROUPS);
        std::hint::black_box(keys.values());
        std::hint::black_box(vals.values());
        warm_kernel.push(measure_nanos(|| {
            let mut ledger = EvidenceLedger::new();
            groupby_sum(&keys, &vals, options, &policy, &mut ledger).expect("warm sum")
        }));

        let (keys, vals) = build_typed_i64_sum_data(N, GROUPS);
        cold_public.push(measure_nanos(|| {
            let mut ledger = EvidenceLedger::new();
            groupby_sum(&keys, &vals, options, &policy, &mut ledger).expect("cold sum")
        }));
    }

    let materialize_p50 = median_nanos(&mut materialize);
    let warm_kernel_p50 = median_nanos(&mut warm_kernel);
    let cold_public_p50 = median_nanos(&mut cold_public);
    println!("typed dense groupby_sum attribution: n={N} groups={GROUPS} samples={SAMPLES}");
    println!("two-column Scalar materialization p50: {materialize_p50} ns");
    println!("warm Scalar groupby_sum p50: {warm_kernel_p50} ns");
    println!("cold public groupby_sum p50: {cold_public_p50} ns");
}

fn former_dense_i64_groupby_min(
    keys: &Series,
    values: &Series,
    policy: &RuntimePolicy,
) -> Option<Series> {
    if keys.index() != values.index() || keys.index().has_duplicates() {
        return None;
    }
    let key_values = keys.values();
    let data_values = values.values();
    let mut ledger = EvidenceLedger::new();
    let _ = policy.decide_join_admission(key_values.len(), &mut ledger);

    if key_values.is_empty() {
        return Series::new(
            "min",
            Index::new(Vec::new()),
            Column::from_values(Vec::new()).ok()?,
        )
        .ok();
    }

    let mut min_key = i64::MAX;
    let mut max_key = i64::MIN;
    for key in key_values {
        let Scalar::Int64(key) = key else {
            return None;
        };
        min_key = min_key.min(*key);
        max_key = max_key.max(*key);
    }
    let span = usize::try_from(i128::from(max_key) - i128::from(min_key) + 1).ok()?;
    if span > 65_536 {
        return None;
    }

    let mut minima = vec![0_i64; span];
    let mut seen = vec![false; span];
    for (key, value) in key_values.iter().zip(data_values) {
        let Scalar::Int64(key) = key else {
            return None;
        };
        let Scalar::Int64(value) = value else {
            return None;
        };
        let bucket = usize::try_from(i128::from(*key) - i128::from(min_key)).ok()?;
        if !seen[bucket] {
            seen[bucket] = true;
            minima[bucket] = *value;
        } else if *value < minima[bucket] {
            minima[bucket] = *value;
        }
    }

    let mut out_index = Vec::with_capacity(span);
    let mut out_values = Vec::with_capacity(span);
    for (bucket, was_seen) in seen.iter().enumerate() {
        if !*was_seen {
            continue;
        }
        let key = min_key.checked_add(i64::try_from(bucket).ok()?)?;
        out_index.push(IndexLabel::Int64(key));
        out_values.push(Scalar::Int64(minima[bucket]));
    }
    Series::new(
        "min",
        Index::new(out_index),
        Column::from_values(out_values).ok()?,
    )
    .ok()
}

fn run_min_ab() {
    const N: usize = 100_000;
    const GROUPS: i64 = 1_000;
    const SAMPLES: usize = 10;
    let started = Instant::now();
    let policy = RuntimePolicy::strict();
    let options = GroupByOptions::default();

    let (former_keys, former_vals) = build_typed_i64_sum_data(N, GROUPS);
    let (candidate_keys, candidate_vals) = build_typed_i64_sum_data(N, GROUPS);
    let former = former_dense_i64_groupby_min(&former_keys, &former_vals, &policy)
        .expect("valid former min input");
    let mut candidate_ledger = EvidenceLedger::new();
    let candidate = groupby_min(
        &candidate_keys,
        &candidate_vals,
        options,
        &policy,
        &mut candidate_ledger,
    )
    .expect("candidate min");
    assert_eq!(candidate.index(), former.index());
    assert_eq!(candidate.values(), former.values());

    for _ in 0..2 {
        let (former_keys, former_vals) = build_typed_i64_sum_data(N, GROUPS);
        let (candidate_keys, candidate_vals) = build_typed_i64_sum_data(N, GROUPS);
        std::hint::black_box(
            former_dense_i64_groupby_min(&former_keys, &former_vals, &policy)
                .expect("valid former warmup input"),
        );
        let mut ledger = EvidenceLedger::new();
        std::hint::black_box(
            groupby_min(
                &candidate_keys,
                &candidate_vals,
                options,
                &policy,
                &mut ledger,
            )
            .expect("candidate warmup"),
        );
    }

    let mut former_a = Vec::with_capacity(SAMPLES);
    let mut former_b = Vec::with_capacity(SAMPLES);
    let mut candidate_a = Vec::with_capacity(SAMPLES);
    let mut candidate_b = Vec::with_capacity(SAMPLES);
    for sample in 0..SAMPLES {
        let (former_a_keys, former_a_vals) = build_typed_i64_sum_data(N, GROUPS);
        let (candidate_a_keys, candidate_a_vals) = build_typed_i64_sum_data(N, GROUPS);
        let (candidate_b_keys, candidate_b_vals) = build_typed_i64_sum_data(N, GROUPS);
        let (former_b_keys, former_b_vals) = build_typed_i64_sum_data(N, GROUPS);

        let mut measure_former_a = || {
            former_a.push(measure_nanos(|| {
                former_dense_i64_groupby_min(&former_a_keys, &former_a_vals, &policy)
                    .expect("valid former A input")
            }));
        };
        let mut measure_former_b = || {
            former_b.push(measure_nanos(|| {
                former_dense_i64_groupby_min(&former_b_keys, &former_b_vals, &policy)
                    .expect("valid former B input")
            }));
        };
        let mut measure_candidate_a = || {
            candidate_a.push(measure_nanos(|| {
                let mut ledger = EvidenceLedger::new();
                groupby_min(
                    &candidate_a_keys,
                    &candidate_a_vals,
                    options,
                    &policy,
                    &mut ledger,
                )
                .expect("candidate A")
            }));
        };
        let mut measure_candidate_b = || {
            candidate_b.push(measure_nanos(|| {
                let mut ledger = EvidenceLedger::new();
                groupby_min(
                    &candidate_b_keys,
                    &candidate_b_vals,
                    options,
                    &policy,
                    &mut ledger,
                )
                .expect("candidate B")
            }));
        };

        if sample.is_multiple_of(2) {
            measure_former_a();
            measure_candidate_a();
            measure_candidate_b();
            measure_former_b();
        } else {
            measure_former_b();
            measure_candidate_b();
            measure_candidate_a();
            measure_former_a();
        }
    }

    let former_a_p50 = median_nanos(&mut former_a);
    let former_b_p50 = median_nanos(&mut former_b);
    let candidate_a_p50 = median_nanos(&mut candidate_a);
    let candidate_b_p50 = median_nanos(&mut candidate_b);
    let former_mean = (former_a_p50 + former_b_p50) as f64 / 2.0;
    let candidate_mean = (candidate_a_p50 + candidate_b_p50) as f64 / 2.0;
    println!("dense Int64 groupby_min A/B: n={N} groups={GROUPS} samples={SAMPLES}");
    println!("former cold Scalar p50 A/B: {former_a_p50} / {former_b_p50} ns");
    println!("candidate raw slices p50 A/B: {candidate_a_p50} / {candidate_b_p50} ns");
    println!(
        "former/candidate duplicate-p50 ratio: {:.6}x",
        former_mean / candidate_mean
    );
    println!("measurement wall: {:.3} s", started.elapsed().as_secs_f64());
}

fn former_dense_i64_groupby_sum(keys: &Series, values: &Series, policy: &RuntimePolicy) -> Series {
    assert_eq!(keys.index(), values.index());
    assert!(!keys.index().has_duplicates());
    let key_values = keys.values();
    let data_values = values.values();
    let mut ledger = EvidenceLedger::new();
    let _ = policy.decide_join_admission(key_values.len(), &mut ledger);

    assert!(
        data_values
            .iter()
            .all(|value| matches!(value, Scalar::Int64(_) | Scalar::Bool(_)))
    );

    if key_values.is_empty() {
        return Series::new(
            "sum",
            Index::new(Vec::new()),
            Column::from_values(Vec::new()).expect("empty former column"),
        )
        .expect("empty former series");
    }

    let mut min_key = i64::MAX;
    let mut max_key = i64::MIN;
    assert!(key_values.iter().all(|key| matches!(key, Scalar::Int64(_))));
    for key in key_values {
        if let Scalar::Int64(key) = key {
            min_key = min_key.min(*key);
            max_key = max_key.max(*key);
        }
    }
    let span = usize::try_from(i128::from(max_key) - i128::from(min_key) + 1)
        .expect("bounded benchmark key span");
    let mut sums = vec![0_i128; span];
    let mut seen = vec![false; span];
    for (key, value) in key_values.iter().zip(data_values) {
        if let Scalar::Int64(key) = key {
            let bucket = usize::try_from(i128::from(*key) - i128::from(min_key))
                .expect("bounded benchmark key");
            seen[bucket] = true;
            match value {
                Scalar::Int64(value) => sums[bucket] += i128::from(*value),
                Scalar::Bool(value) => sums[bucket] += i128::from(*value),
                _ => {}
            }
        }
    }

    let mut out_index = Vec::with_capacity(span);
    let mut out_values = Vec::with_capacity(span);
    for (bucket, was_seen) in seen.iter().enumerate() {
        if !*was_seen {
            continue;
        }
        let key = min_key + i64::try_from(bucket).expect("bounded bucket");
        out_index.push(IndexLabel::Int64(key));
        out_values.push(match i64::try_from(sums[bucket]) {
            Ok(value) => Scalar::Int64(value),
            Err(_) => Scalar::Float64(sums[bucket] as f64),
        });
    }
    Series::new(
        "sum",
        Index::new(out_index),
        Column::from_values(out_values).expect("former output column"),
    )
    .expect("former output series")
}

fn run_sum_ab() {
    const N: usize = 100_000;
    const GROUPS: i64 = 1_000;
    const SAMPLES: usize = 15;
    let policy = RuntimePolicy::strict();
    let options = GroupByOptions::default();

    let (former_keys, former_vals) = build_typed_i64_sum_data(N, GROUPS);
    let (candidate_keys, candidate_vals) = build_typed_i64_sum_data(N, GROUPS);
    let former = former_dense_i64_groupby_sum(&former_keys, &former_vals, &policy);
    let mut candidate_ledger = EvidenceLedger::new();
    let candidate = groupby_sum(
        &candidate_keys,
        &candidate_vals,
        options,
        &policy,
        &mut candidate_ledger,
    )
    .expect("candidate sum");
    assert_eq!(candidate.index(), former.index());
    assert_eq!(candidate.values(), former.values());

    for _ in 0..3 {
        let (former_keys, former_vals) = build_typed_i64_sum_data(N, GROUPS);
        let (candidate_keys, candidate_vals) = build_typed_i64_sum_data(N, GROUPS);
        std::hint::black_box(former_dense_i64_groupby_sum(
            &former_keys,
            &former_vals,
            &policy,
        ));
        let mut ledger = EvidenceLedger::new();
        std::hint::black_box(
            groupby_sum(
                &candidate_keys,
                &candidate_vals,
                options,
                &policy,
                &mut ledger,
            )
            .expect("candidate warmup"),
        );
    }

    let mut former_a = Vec::with_capacity(SAMPLES);
    let mut former_b = Vec::with_capacity(SAMPLES);
    let mut candidate_a = Vec::with_capacity(SAMPLES);
    let mut candidate_b = Vec::with_capacity(SAMPLES);
    for sample in 0..SAMPLES {
        let (former_a_keys, former_a_vals) = build_typed_i64_sum_data(N, GROUPS);
        let (candidate_a_keys, candidate_a_vals) = build_typed_i64_sum_data(N, GROUPS);
        let (candidate_b_keys, candidate_b_vals) = build_typed_i64_sum_data(N, GROUPS);
        let (former_b_keys, former_b_vals) = build_typed_i64_sum_data(N, GROUPS);

        let mut measure_former_a = || {
            former_a.push(measure_nanos(|| {
                former_dense_i64_groupby_sum(&former_a_keys, &former_a_vals, &policy)
            }));
        };
        let mut measure_former_b = || {
            former_b.push(measure_nanos(|| {
                former_dense_i64_groupby_sum(&former_b_keys, &former_b_vals, &policy)
            }));
        };
        let mut measure_candidate_a = || {
            candidate_a.push(measure_nanos(|| {
                let mut ledger = EvidenceLedger::new();
                groupby_sum(
                    &candidate_a_keys,
                    &candidate_a_vals,
                    options,
                    &policy,
                    &mut ledger,
                )
                .expect("candidate A")
            }));
        };
        let mut measure_candidate_b = || {
            candidate_b.push(measure_nanos(|| {
                let mut ledger = EvidenceLedger::new();
                groupby_sum(
                    &candidate_b_keys,
                    &candidate_b_vals,
                    options,
                    &policy,
                    &mut ledger,
                )
                .expect("candidate B")
            }));
        };

        if sample.is_multiple_of(2) {
            measure_former_a();
            measure_candidate_a();
            measure_candidate_b();
            measure_former_b();
        } else {
            measure_former_b();
            measure_candidate_b();
            measure_candidate_a();
            measure_former_a();
        }
    }

    let former_a_p50 = median_nanos(&mut former_a);
    let former_b_p50 = median_nanos(&mut former_b);
    let candidate_a_p50 = median_nanos(&mut candidate_a);
    let candidate_b_p50 = median_nanos(&mut candidate_b);
    let former_mean = (former_a_p50 + former_b_p50) as f64 / 2.0;
    let candidate_mean = (candidate_a_p50 + candidate_b_p50) as f64 / 2.0;
    println!("dense Int64 groupby_sum A/B: n={N} groups={GROUPS} samples={SAMPLES}");
    println!("former cold Scalar p50 A/B: {former_a_p50} / {former_b_p50} ns");
    println!("candidate raw slices p50 A/B: {candidate_a_p50} / {candidate_b_p50} ns");
    println!(
        "former/candidate duplicate-p50 ratio: {:.6}x",
        former_mean / candidate_mean
    );
}

fn former_dense_i64_groupby_mean(
    keys: &Series,
    values: &Series,
    policy: &RuntimePolicy,
) -> Option<Series> {
    assert_eq!(keys.index(), values.index());
    assert!(!keys.index().has_duplicates());
    let key_values = keys.values();
    let data_values = values.values();
    let mut ledger = EvidenceLedger::new();
    let _ = policy.decide_join_admission(key_values.len(), &mut ledger);

    if key_values.is_empty() {
        return Series::new(
            "mean",
            Index::new(Vec::new()),
            Column::from_values(Vec::new()).ok()?,
        )
        .ok();
    }

    let mut min_key = i64::MAX;
    let mut max_key = i64::MIN;
    for key in key_values {
        let Scalar::Int64(key) = key else {
            return None;
        };
        min_key = min_key.min(*key);
        max_key = max_key.max(*key);
    }
    let span = usize::try_from(i128::from(max_key) - i128::from(min_key) + 1).ok()?;
    if span > 65_536 {
        return None;
    }

    let mut sums = vec![0.0_f64; span];
    let mut counts = vec![0_i64; span];
    let mut seen = vec![false; span];
    for (key, value) in key_values.iter().zip(data_values) {
        let Scalar::Int64(key) = key else {
            return None;
        };
        let Scalar::Int64(value) = value else {
            return None;
        };
        let bucket = usize::try_from(i128::from(*key) - i128::from(min_key)).ok()?;
        seen[bucket] = true;
        sums[bucket] += *value as f64;
        counts[bucket] += 1;
    }

    let mut out_index = Vec::with_capacity(span);
    let mut out_values = Vec::with_capacity(span);
    for (bucket, was_seen) in seen.iter().enumerate() {
        if !*was_seen {
            continue;
        }
        let key = min_key.checked_add(i64::try_from(bucket).ok()?)?;
        out_index.push(IndexLabel::Int64(key));
        out_values.push(Scalar::Float64(sums[bucket] / counts[bucket] as f64));
    }
    Series::new(
        "mean",
        Index::new(out_index),
        Column::from_values(out_values).ok()?,
    )
    .ok()
}

fn run_mean_ab() {
    const N: usize = 100_000;
    const GROUPS: i64 = 1_000;
    const SAMPLES: usize = 10;
    let started = Instant::now();
    let policy = RuntimePolicy::strict();
    let options = GroupByOptions::default();

    let (former_keys, former_vals) = build_typed_i64_sum_data(N, GROUPS);
    let (candidate_keys, candidate_vals) = build_typed_i64_sum_data(N, GROUPS);
    let former = former_dense_i64_groupby_mean(&former_keys, &former_vals, &policy)
        .expect("valid former mean input");
    let mut candidate_ledger = EvidenceLedger::new();
    let candidate = groupby_mean(
        &candidate_keys,
        &candidate_vals,
        options,
        &policy,
        &mut candidate_ledger,
    )
    .expect("candidate mean");
    assert_eq!(candidate.index(), former.index());
    assert_eq!(candidate.values(), former.values());

    for _ in 0..2 {
        let (former_keys, former_vals) = build_typed_i64_sum_data(N, GROUPS);
        let (candidate_keys, candidate_vals) = build_typed_i64_sum_data(N, GROUPS);
        std::hint::black_box(
            former_dense_i64_groupby_mean(&former_keys, &former_vals, &policy)
                .expect("valid former warmup input"),
        );
        let mut ledger = EvidenceLedger::new();
        std::hint::black_box(
            groupby_mean(
                &candidate_keys,
                &candidate_vals,
                options,
                &policy,
                &mut ledger,
            )
            .expect("candidate warmup"),
        );
    }

    let mut former_a = Vec::with_capacity(SAMPLES);
    let mut former_b = Vec::with_capacity(SAMPLES);
    let mut candidate_a = Vec::with_capacity(SAMPLES);
    let mut candidate_b = Vec::with_capacity(SAMPLES);
    for sample in 0..SAMPLES {
        let (former_a_keys, former_a_vals) = build_typed_i64_sum_data(N, GROUPS);
        let (candidate_a_keys, candidate_a_vals) = build_typed_i64_sum_data(N, GROUPS);
        let (candidate_b_keys, candidate_b_vals) = build_typed_i64_sum_data(N, GROUPS);
        let (former_b_keys, former_b_vals) = build_typed_i64_sum_data(N, GROUPS);

        let mut measure_former_a = || {
            former_a.push(measure_nanos(|| {
                former_dense_i64_groupby_mean(&former_a_keys, &former_a_vals, &policy)
                    .expect("valid former A input")
            }));
        };
        let mut measure_former_b = || {
            former_b.push(measure_nanos(|| {
                former_dense_i64_groupby_mean(&former_b_keys, &former_b_vals, &policy)
                    .expect("valid former B input")
            }));
        };
        let mut measure_candidate_a = || {
            candidate_a.push(measure_nanos(|| {
                let mut ledger = EvidenceLedger::new();
                groupby_mean(
                    &candidate_a_keys,
                    &candidate_a_vals,
                    options,
                    &policy,
                    &mut ledger,
                )
                .expect("candidate A")
            }));
        };
        let mut measure_candidate_b = || {
            candidate_b.push(measure_nanos(|| {
                let mut ledger = EvidenceLedger::new();
                groupby_mean(
                    &candidate_b_keys,
                    &candidate_b_vals,
                    options,
                    &policy,
                    &mut ledger,
                )
                .expect("candidate B")
            }));
        };

        if sample.is_multiple_of(2) {
            measure_former_a();
            measure_candidate_a();
            measure_candidate_b();
            measure_former_b();
        } else {
            measure_former_b();
            measure_candidate_b();
            measure_candidate_a();
            measure_former_a();
        }
    }

    let former_a_p50 = median_nanos(&mut former_a);
    let former_b_p50 = median_nanos(&mut former_b);
    let candidate_a_p50 = median_nanos(&mut candidate_a);
    let candidate_b_p50 = median_nanos(&mut candidate_b);
    let former_mean = (former_a_p50 + former_b_p50) as f64 / 2.0;
    let candidate_mean = (candidate_a_p50 + candidate_b_p50) as f64 / 2.0;
    println!("dense Int64 groupby_mean A/B: n={N} groups={GROUPS} samples={SAMPLES}");
    println!("former cold Scalar p50 A/B: {former_a_p50} / {former_b_p50} ns");
    println!("candidate raw slices p50 A/B: {candidate_a_p50} / {candidate_b_p50} ns");
    println!(
        "former/candidate duplicate-p50 ratio: {:.6}x",
        former_mean / candidate_mean
    );
    println!("measurement wall: {:.3} s", started.elapsed().as_secs_f64());
}

/// `cargo bench` entry point. The unstable libtest bencher reports the median
/// sample time, which makes this the decision surface for dispatch changes.
#[cfg(test)]
#[bench]
fn dense_int64_median_dispatch(b: &mut test::Bencher) {
    let (keys, vals) = build_data(2_000_000, 200);
    let policy = RuntimePolicy::strict();

    b.iter(|| {
        let mut ledger = EvidenceLedger::new();
        let out = groupby_agg(
            &keys,
            &vals,
            AggFunc::Median,
            GroupByOptions::default(),
            &policy,
            &mut ledger,
        )
        .expect("median");
        test::black_box(out);
    });
}

/// Median-gated decision surface for the dense two-pass variance kernel.
#[cfg(test)]
#[bench]
fn dense_int64_var_second_pass(b: &mut test::Bencher) {
    let (keys, vals) = build_data(2_000_000, 200);
    let policy = RuntimePolicy::strict();

    b.iter(|| {
        let mut ledger = EvidenceLedger::new();
        let out = groupby_agg(
            &keys,
            &vals,
            AggFunc::Var,
            GroupByOptions::default(),
            &policy,
            &mut ledger,
        )
        .expect("var");
        test::black_box(out);
    });
}

fn main() {
    let mode = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "bench".to_string());
    if mode == "sum-attribution" {
        run_sum_attribution();
        return;
    }
    if mode == "sum-ab" {
        run_sum_ab();
        return;
    }
    if mode == "min-ab" {
        run_min_ab();
        return;
    }
    if mode == "mean-ab" {
        run_mean_ab();
        return;
    }
    let n: usize = 2_000_000;
    let num_groups: i64 = 200;
    let (keys, vals) = build_data(n, num_groups);
    let policy = RuntimePolicy::strict();

    if mode == "golden" {
        let mut ledger = EvidenceLedger::new();
        let out = groupby_agg(
            &keys,
            &vals,
            AggFunc::Median,
            GroupByOptions::default(),
            &policy,
            &mut ledger,
        )
        .expect("median");
        for (label, val) in out.index().labels().iter().zip(out.values().iter()) {
            let bits = match val {
                Scalar::Float64(f) => f.to_bits(),
                _ => 0,
            };
            println!("{label:?}\t{bits:016x}");
        }
        return;
    }

    // bench
    let iters = 20;
    let mut best = f64::INFINITY;
    for _ in 0..iters {
        let mut ledger = EvidenceLedger::new();
        let t0 = Instant::now();
        let out = groupby_agg(
            &keys,
            &vals,
            AggFunc::Median,
            GroupByOptions::default(),
            &policy,
            &mut ledger,
        )
        .expect("median");
        let dt = t0.elapsed().as_secs_f64();
        std::hint::black_box(&out);
        if dt < best {
            best = dt;
        }
    }
    println!(
        "groupby median: n={n} groups={num_groups} best={:.3} ms",
        best * 1e3
    );
}
