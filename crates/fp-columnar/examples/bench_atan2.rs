use fp_columnar::Column;
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(5_000_000);
    let op = a.get(2).map(String::as_str).unwrap_or("atan2");
    let x = Column::from_f64_values((0..n).map(|i| (i % 1000) as f64 * 0.01 - 5.0).collect());
    let y = Column::from_f64_values((0..n).map(|i| (i % 777) as f64 * 0.01 - 3.0).collect());
    let mut best = u128::MAX;
    for _ in 0..6 {
        let t = std::time::Instant::now();
        let r = if op == "hypot" {
            x.hypot(&y)
        } else {
            x.atan2(&y)
        }
        .unwrap();
        std::hint::black_box(r.len());
        best = best.min(t.elapsed().as_nanos());
    }
    println!("{op} n={n}: best={best}ns ({:.2}ms)", best as f64 / 1e6);
}
