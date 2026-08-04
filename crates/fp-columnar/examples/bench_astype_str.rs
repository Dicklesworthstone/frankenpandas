//! Column::astype(Utf8) over a 1M Int64 column. bench_astype_str <n>
use fp_columnar::Column;
use fp_types::DType;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(1_000_000);
    // Mixed magnitudes/signs so digit counts vary like real data.
    let data: Vec<i64> = (0..n as i64)
        .map(|i| (i.wrapping_mul(2_654_435_761)) % 2_000_000_000 - 1_000_000_000)
        .collect();
    let col = Column::from_i64_values(data);
    let mut best = u128::MAX;
    for _ in 0..6 {
        let t = std::time::Instant::now();
        std::hint::black_box(col.astype(DType::Utf8).unwrap());
        let e = t.elapsed().as_nanos();
        if e < best {
            best = e;
        }
    }
    println!(
        "astype Int64->Utf8 n={n}: best={best}ns ({:.2}ms)",
        best as f64 / 1e6
    );
}
