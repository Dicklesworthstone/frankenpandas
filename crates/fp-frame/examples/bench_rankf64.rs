use fp_columnar::Column;
use fp_frame::Series;
use fp_index::Index;
fn main() {
    let g: Vec<String> = std::env::args().collect();
    let n: usize = g.get(1).and_then(|s| s.parse().ok()).unwrap_or(5_000_000);
    // shuffled-ish unique f64 (splitmix to avoid monotonic pitfall)
    let col = Column::from_f64_values(
        (0..n as u64)
            .map(|i| {
                let mut z = i.wrapping_mul(0x9E3779B97F4A7C15);
                z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
                (z >> 11) as f64
            })
            .collect(),
    );
    let s = Series::new("s", Index::from_range(0, n as i64, 1), col).unwrap();
    let mut best = u128::MAX;
    for _ in 0..6 {
        let t = std::time::Instant::now();
        let r = s.rank("average", true, "keep").unwrap();
        std::hint::black_box(r.len());
        best = best.min(t.elapsed().as_nanos());
    }
    println!(
        "rank(average) f64 {}M: best={:.2}ms",
        n / 1_000_000,
        best as f64 / 1e6
    );
}
