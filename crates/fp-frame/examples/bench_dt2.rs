//! Series.dt component over 5M datetime. bench_dt2 <n> <op>
use fp_columnar::Column;
use fp_frame::Series;
use fp_index::Index;
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(5_000_000);
    let op = a.get(2).map(String::as_str).unwrap_or("year");
    let base: i64 = 1_600_000_000_000_000_000;
    let col = Column::from_datetime64_values(
        (0..n as i64)
            .map(|i| base + i * 3_600_000_000_000)
            .collect(),
    );
    let s = Series::new("t", Index::from_range(0, n as i64, 1), col).unwrap();
    let mut best = u128::MAX;
    for _ in 0..6 {
        let t = std::time::Instant::now();
        let r = match op {
            "month" => s.dt().month(),
            "dayofweek" => s.dt().dayofweek(),
            _ => s.dt().year(),
        }
        .unwrap();
        std::hint::black_box(r.len());
        best = best.min(t.elapsed().as_nanos());
    }
    println!("dt.{op} n={n}: best={best}ns ({:.2}ms)", best as f64 / 1e6);
}
