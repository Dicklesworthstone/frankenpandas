//! Column skew/kurt over 5M f64. bench_skew <n> <op>
use fp_columnar::Column;
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(5_000_000);
    let op = a.get(2).map(String::as_str).unwrap_or("skew");
    let col = Column::from_f64_values((0..n).map(|i| (i % 10000) as f64 * 0.1).collect());
    let mut best = u128::MAX;
    for _ in 0..6 {
        let t = std::time::Instant::now();
        let r = if op == "kurt" { col.kurt() } else { col.skew() };
        std::hint::black_box(&r);
        best = best.min(t.elapsed().as_nanos());
    }
    println!("{op} f64 n={n}: best={best}ns ({:.2}ms)", best as f64 / 1e6);
}
