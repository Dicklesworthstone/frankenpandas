//! Series.dt().strftime("%Y-%m-%d") over a 1M Datetime64 series. bench_strftime <n>
use fp_columnar::Column;
use fp_frame::Series;
use fp_index::Index;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(1_000_000);
    let fmt = a.get(2).map(String::as_str).unwrap_or("%Y-%m-%d");
    let base = 946_684_800_000_000_000i64; // 2000-01-01
    let step = 37_000_000_000i64; // 37s
    let nanos: Vec<i64> = (0..n as i64).map(|i| base + i * step).collect();
    let s = Series::new(
        "t",
        Index::new_known_unique_int64_unit_range(0, n),
        Column::from_datetime64_values(nanos),
    )
    .unwrap();
    let mut best = u128::MAX;
    for _ in 0..6 {
        let t = std::time::Instant::now();
        std::hint::black_box(s.dt().strftime(fmt).unwrap());
        let e = t.elapsed().as_nanos();
        if e < best {
            best = e;
        }
    }
    println!(
        "strftime '{fmt}' n={n}: best={best}ns ({:.2}ms)",
        best as f64 / 1e6
    );
}
