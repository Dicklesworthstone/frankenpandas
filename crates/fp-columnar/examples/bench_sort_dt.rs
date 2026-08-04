//! Column::sort_values over a 5M Datetime64 column. bench_sort_dt <n>
use fp_columnar::Column;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(5_000_000);
    let data: Vec<i64> = (0..n as i64)
        .map(|i| (i.wrapping_mul(2_654_435_761)).rem_euclid(1_000_000_000_000_000))
        .collect();
    let col = Column::from_datetime64_values(data);
    let mut best = u128::MAX;
    for _ in 0..6 {
        let t = std::time::Instant::now();
        let r = col.sort_values(true).unwrap();
        std::hint::black_box(r.len());
        let e = t.elapsed().as_nanos();
        if e < best {
            best = e;
        }
    }
    println!(
        "sort_values datetime n={n}: best={best}ns ({:.2}ms)",
        best as f64 / 1e6
    );
}
