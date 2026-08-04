//! Column pow (col ** col) over 5M f64. bench_pow <n>
use fp_columnar::Column;
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(5_000_000);
    let base = Column::from_f64_values((0..n).map(|i| (i % 1000) as f64 * 0.01 + 0.5).collect());
    let exp = Column::from_f64_values(vec![2.5; n]);
    let mut best = u128::MAX;
    for _ in 0..6 {
        let t = std::time::Instant::now();
        let r = base.pow(&exp).unwrap();
        std::hint::black_box(r.len());
        best = best.min(t.elapsed().as_nanos());
    }
    println!(
        "pow col**col n={n}: best={best}ns ({:.2}ms)",
        best as f64 / 1e6
    );
}
