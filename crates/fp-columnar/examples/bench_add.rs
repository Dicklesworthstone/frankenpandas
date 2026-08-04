//! Column::add col+col over 5M. bench_add <n> <dt>
use fp_columnar::Column;
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(5_000_000);
    let dt = a.get(2).map(String::as_str).unwrap_or("f64");
    let (x, y) = if dt == "i64" {
        (
            Column::from_i64_values((0..n as i64).map(|i| i % 1000).collect()),
            Column::from_i64_values((0..n as i64).map(|i| i % 777).collect()),
        )
    } else {
        (
            Column::from_f64_values((0..n).map(|i| (i % 1000) as f64 * 0.5).collect()),
            Column::from_f64_values((0..n).map(|i| (i % 777) as f64 * 0.3).collect()),
        )
    };
    let mut best = u128::MAX;
    for _ in 0..6 {
        let t = std::time::Instant::now();
        let r = x.add(&y).unwrap();
        std::hint::black_box(r.len());
        best = best.min(t.elapsed().as_nanos());
    }
    println!("add {dt} n={n}: best={best}ns ({:.2}ms)", best as f64 / 1e6);
}
