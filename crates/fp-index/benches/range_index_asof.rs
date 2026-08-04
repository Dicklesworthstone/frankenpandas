use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, criterion_group};
use fp_index::{Index, IndexLabel, RangeIndex};
use sha2::{Digest, Sha256};

const SIZES: &[usize] = &[100_000, 1_000_000];
const PROBE_COUNT: usize = 4_096;

/// SHA-256 of the executable that is about to run the benchmark.
fn self_identity() -> String {
    use std::fmt::Write as _;

    let Ok(path) = std::env::current_exe() else {
        return "unavailable".to_string();
    };
    let Ok(bytes) = std::fs::read(&path) else {
        return "unavailable".to_string();
    };
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    let digest = hasher.finalize();
    let mut sha256 = String::with_capacity(digest.len() * 2);
    for byte in digest {
        write!(sha256, "{byte:02x}").expect("writing to String cannot fail");
    }
    format!("{} ({} bytes) {}", sha256, bytes.len(), path.display())
}

fn build_range(size: usize) -> RangeIndex {
    RangeIndex::new(0, (size as i64) * 2, 2).expect("valid ascending range")
}

fn build_affine_index(size: usize) -> Index {
    Index::new_known_unique_int64_affine_range(0, 2, size).expect("valid ascending affine index")
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

fn affine_index_asof_checksum(index: &Index, probes: &[IndexLabel]) -> i64 {
    probes
        .iter()
        .map(|probe| match index.asof(black_box(probe)) {
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

fn bench_affine_index_asof(c: &mut Criterion) {
    let mut group = c.benchmark_group("affine_index_asof");
    for &size in SIZES {
        let index = build_affine_index(size);
        let probes = build_probes(size);
        group.bench_with_input(BenchmarkId::from_parameter(size), &size, |b, _| {
            b.iter(|| {
                black_box(affine_index_asof_checksum(
                    black_box(&index),
                    black_box(&probes),
                ))
            });
        });
    }
    group.finish();
}

criterion_group!(benches, bench_range_index_asof, bench_affine_index_asof);

fn main() {
    println!("bench_elf_sha256={}", self_identity());
    benches();
    Criterion::default().configure_from_args().final_summary();
}
