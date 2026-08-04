//! Column::astype int->float over 5M. bench_astype <n>
use fp_columnar::Column;
use fp_types::DType;
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(5_000_000);
    let col = Column::from_i64_values((0..n as i64).map(|i| i % 100000).collect());
    let mut best = u128::MAX;
    for _ in 0..6 {
        let t = std::time::Instant::now();
        let r = col.astype(DType::Float64).unwrap();
        std::hint::black_box(r.len());
        let e = t.elapsed().as_nanos();
        if e < best {
            best = e;
        }
    }
    println!(
        "astype i64->f64 n={n}: best={best}ns ({:.2}ms)",
        best as f64 / 1e6
    );
}
