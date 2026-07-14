use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use fp_index::{IndexLabel, RangeIndex};

const SIZES: &[usize] = &[100_000, 1_000_000];

fn build_source(size: usize) -> RangeIndex {
    RangeIndex::new(0, (size as i64) * 2, 2).expect("valid source range")
}

fn build_miss_heavy_targets(size: usize) -> Vec<i64> {
    (0..size)
        .map(|i| {
            if i % 16 == 0 {
                ((i % size) as i64) * 2
            } else {
                ((i as i64) * 2) + 1
            }
        })
        .collect()
}

fn build_all_miss_target_range(size: usize) -> RangeIndex {
    RangeIndex::new(1, (size as i64) * 2 + 1, 2).expect("valid target range")
}

fn build_all_miss_targets(size: usize) -> Vec<i64> {
    (0..size).map(|i| (i as i64) * 2 + 1).collect()
}

fn checksum_indexer(indexer: &[isize]) -> isize {
    indexer.iter().fold(indexer.len() as isize, |acc, value| {
        acc.wrapping_add(*value)
    })
}

fn current_get_indexer_checksum(source: &RangeIndex, targets: &[i64]) -> isize {
    checksum_indexer(&source.get_indexer(targets))
}

fn legacy_get_loc_indexer_checksum(source: &RangeIndex, targets: &[i64]) -> isize {
    let mut checksum = targets.len() as isize;
    for &target in targets {
        let position = source
            .get_loc(target)
            .map_or(-1, |position| position as isize);
        checksum = checksum.wrapping_add(position);
    }
    checksum
}

fn current_reindex_checksum(source: &RangeIndex, target: &RangeIndex) -> isize {
    let (_, indexer) = source.reindex(target);
    checksum_indexer(&indexer)
}

fn checksum_i64_values(values: &[i64]) -> i64 {
    values
        .iter()
        .fold(values.len() as i64, |acc, value| acc.wrapping_add(*value))
}

fn current_values_checksum(source: &RangeIndex) -> i64 {
    checksum_i64_values(&source.values())
}

fn legacy_flat_index_values_checksum(source: &RangeIndex) -> i64 {
    let values: Vec<i64> = source
        .to_index()
        .labels()
        .iter()
        .map(|label| match label {
            IndexLabel::Int64(value) => *value,
            other => panic!("RangeIndex materialized non-int64 label: {other:?}"),
        })
        .collect();
    checksum_i64_values(&values)
}

fn legacy_get_loc_reindex_checksum(source: &RangeIndex, targets: &[i64]) -> isize {
    let mut checksum = targets.len() as isize;
    for &value in targets {
        let source_position = source
            .get_loc(value)
            .map_or(-1, |position| position as isize);
        checksum = checksum.wrapping_add(source_position);
    }
    checksum
}

fn former_range_prod(source: &RangeIndex) -> i64 {
    let mut total: i128 = 1;
    for position in 0..source.len() {
        let value = i128::from(source.start()) + (position as i128) * i128::from(source.step());
        total = total.saturating_mul(value);
    }
    i64::try_from(total).unwrap_or(if total > 0 { i64::MAX } else { i64::MIN })
}

fn bench_range_index_indexers(c: &mut Criterion) {
    let mut group = c.benchmark_group("range_index_indexers");
    for &size in SIZES {
        let source = build_source(size);
        let targets = build_miss_heavy_targets(size);
        let target_range = build_all_miss_target_range(size);
        let target_range_values = build_all_miss_targets(size);

        group.bench_with_input(
            BenchmarkId::new("current_get_indexer_miss_heavy", size),
            &size,
            |b, _| {
                b.iter(|| {
                    black_box(current_get_indexer_checksum(
                        black_box(&source),
                        black_box(&targets),
                    ));
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("legacy_get_loc_loop_miss_heavy", size),
            &size,
            |b, _| {
                b.iter(|| {
                    black_box(legacy_get_loc_indexer_checksum(
                        black_box(&source),
                        black_box(&targets),
                    ));
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("current_reindex_all_miss", size),
            &size,
            |b, _| {
                b.iter(|| {
                    black_box(current_reindex_checksum(
                        black_box(&source),
                        black_box(&target_range),
                    ));
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("legacy_get_loc_reindex_all_miss", size),
            &size,
            |b, _| {
                b.iter(|| {
                    black_box(legacy_get_loc_reindex_checksum(
                        black_box(&source),
                        black_box(&target_range_values),
                    ));
                });
            },
        );
    }
    group.finish();
}

fn bench_range_index_values(c: &mut Criterion) {
    let mut group = c.benchmark_group("range_index_values");
    for &size in SIZES {
        let source = build_source(size);

        group.bench_with_input(
            BenchmarkId::new("current_direct_values", size),
            &size,
            |b, _| {
                b.iter(|| {
                    black_box(current_values_checksum(black_box(&source)));
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("legacy_flat_index_values", size),
            &size,
            |b, _| {
                b.iter(|| {
                    black_box(legacy_flat_index_values_checksum(black_box(&source)));
                });
            },
        );
    }
    group.finish();
}

fn bench_range_index_prod(c: &mut Criterion) {
    let source = RangeIndex::new(1, 10_001, 1).expect("valid positive range");
    assert_eq!(source.prod(), former_range_prod(&source));

    let mut group = c.benchmark_group("range_index_prod");
    group.bench_function("former_full_scan_10000", |b| {
        b.iter(|| black_box(former_range_prod(black_box(&source))));
    });
    group.bench_function("candidate_positive_saturation_10000", |b| {
        b.iter(|| black_box(black_box(&source).prod()));
    });
    group.finish();
}

criterion_group!(
    benches,
    bench_range_index_indexers,
    bench_range_index_values,
    bench_range_index_prod
);
criterion_main!(benches);
