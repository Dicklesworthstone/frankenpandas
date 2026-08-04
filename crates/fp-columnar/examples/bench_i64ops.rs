//! astype f64->i64 over 5M. bench_i64ops <n>
use fp_columnar::Column;
use fp_types::DType;
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(5_000_000);
    let col = Column::from_f64_values((0..n).map(|i| (i % 100000) as f64).collect());
    let mut best = u128::MAX;
    for _ in 0..6 {
        let t = std::time::Instant::now();
        let r = col.astype(DType::Int64).unwrap();
        std::hint::black_box(r.len());
        best = best.min(t.elapsed().as_nanos());
    }
    println!(
        "astype f64->i64 n={n}: best={best}ns ({:.2}ms)",
        best as f64 / 1e6
    );
}
