//! Column::pct_change over a 5M column. bench_pctchange <n> <dt>
use fp_columnar::Column;
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(5_000_000);
    let dt = a.get(2).map(String::as_str).unwrap_or("f64");
    let col = if dt == "i64" {
        Column::from_i64_values((0..n as i64).map(|i| (i % 1000) + 1).collect())
    } else {
        Column::from_f64_values((0..n).map(|i| (i % 1000) as f64 + 1.5).collect())
    };
    let mut best = u128::MAX;
    for _ in 0..6 {
        let t = std::time::Instant::now();
        let r = col.pct_change(1).unwrap();
        std::hint::black_box(r.len());
        let e = t.elapsed().as_nanos();
        if e < best {
            best = e;
        }
    }
    println!(
        "pct_change {dt} n={n}: best={best}ns ({:.2}ms)",
        best as f64 / 1e6
    );
}
