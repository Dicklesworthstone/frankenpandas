use fp_columnar::Column;
fn main() {
    let g: Vec<String> = std::env::args().collect();
    let n: usize = g.get(1).and_then(|s| s.parse().ok()).unwrap_or(5_000_000);
    let a = Column::from_f64_values((0..n).map(|i| (i % 1000) as f64 * 0.5).collect());
    let b = Column::from_f64_values((0..n).map(|i| (i % 777) as f64 * 0.3).collect());
    let mut best = u128::MAX;
    for _ in 0..6 {
        let t = std::time::Instant::now();
        let r = a.maximum(&b).unwrap();
        std::hint::black_box(r.len());
        best = best.min(t.elapsed().as_nanos());
    }
    println!("maximum 5M: best={best}ns ({:.2}ms)", best as f64 / 1e6);
}
