use fp_columnar::Column;
fn main() {
    let g: Vec<String> = std::env::args().collect();
    let n: usize = g.get(1).and_then(|s| s.parse().ok()).unwrap_or(5_000_000);
    // ~50% selective mask
    let data = Column::from_i64_values((0..n as i64).collect());
    let dataf = Column::from_f64_values((0..n).map(|i| i as f64 * 1.5).collect());
    let mask = Column::from_bool_values((0..n).map(|i| i % 2 == 0).collect());
    let mut best = u128::MAX;
    for _ in 0..6 {
        let t = std::time::Instant::now();
        let r = data.filter_by_mask(&mask).unwrap();
        std::hint::black_box(r.len());
        best = best.min(t.elapsed().as_nanos());
    }
    println!(
        "filter_by_mask i64 {}M (50%): best={:.2}ms",
        n / 1_000_000,
        best as f64 / 1e6
    );
    let mut best = u128::MAX;
    for _ in 0..6 {
        let t = std::time::Instant::now();
        let r = dataf.filter_by_mask(&mask).unwrap();
        std::hint::black_box(r.len());
        best = best.min(t.elapsed().as_nanos());
    }
    println!(
        "filter_by_mask f64 {}M (50%): best={:.2}ms",
        n / 1_000_000,
        best as f64 / 1e6
    );
}
