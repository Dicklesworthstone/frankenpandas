use fp_columnar::Column;
fn main() {
    let g: Vec<String> = std::env::args().collect();
    let n: usize = g.get(1).and_then(|s| s.parse().ok()).unwrap_or(5_000_000);
    let op = g.get(2).map(String::as_str).unwrap_or("mod");
    let a = Column::from_i64_values((0..n as i64).map(|i| i % 100000 + 1).collect());
    let b = Column::from_i64_values((0..n as i64).map(|i| i % 97 + 1).collect());
    let mut best = u128::MAX;
    for _ in 0..6 {
        let t = std::time::Instant::now();
        let r = if op == "floordiv" {
            a.floordiv(&b)
        } else {
            a.r#mod(&b)
        }
        .unwrap();
        std::hint::black_box(r.len());
        best = best.min(t.elapsed().as_nanos());
    }
    println!("i64 {op} 5M: best={best}ns ({:.2}ms)", best as f64 / 1e6);
}
