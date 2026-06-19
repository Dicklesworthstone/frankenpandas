use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use fp_index::{IndexLabel, RangeIndex};

const SIZES: &[usize] = &[100_000, 1_000_000];
const PROBE_COUNT: usize = 4_096;

fn build_range(size: usize) -> RangeIndex {
    RangeIndex::new(0, (size as i64) * 2, 2).expect("valid ascending range")
}

fn build_probes(size: usize) -> Vec<IndexLabel> {
    let span = (size as i64) * 2 + 256;
    (0..PROBE_COUNT)
        .map(|i| {
            let mixed = ((i as i64 * 65_537) % span) - 128;
            IndexLabel::Int64(mixed)
        })
        .collect()
}

fn asof_checksum(range: &RangeIndex, probes: &[IndexLabel]) -> i64 {
    probes
        .iter()
        .map(|probe| match range.asof(black_box(probe)) {
            Some(IndexLabel::Int64(value)) => value,
            Some(_) | None => -1,
        })
        .sum()
}

fn bench_range_index_asof(c: &mut Criterion) {
    let mut group = c.benchmark_group("range_index_asof");
    for &size in SIZES {
        let range = build_range(size);
        let probes = build_probes(size);
        group.bench_with_input(BenchmarkId::from_parameter(size), &size, |b, _| {
            b.iter(|| black_box(asof_checksum(black_box(&range), black_box(&probes))));
        });
    }
    group.finish();
}

criterion_group!(benches, bench_range_index_asof);
criterion_main!(benches);
