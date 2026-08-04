//! Series.to_period(freq) over a 1M Datetime64 series. bench_toperiod <n> <freq>
use fp_columnar::Column;
use fp_frame::Series;
use fp_index::{Index, IndexLabel};

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(1_000_000);
    let freq = a.get(2).map(String::as_str).unwrap_or("M");
    let base = 1_577_836_800_000_000_000i64;
    let hour = 3_600_000_000_000i64;
    // Datetime64 INDEX (fp.to_period converts the row index to a PeriodIndex).
    let labels: Vec<IndexLabel> = (0..n)
        .map(|i| IndexLabel::Datetime64(base + (i as i64 % 90000) * hour))
        .collect();
    let s = Series::new(
        "t",
        Index::new(labels),
        Column::from_f64_values((0..n).map(|i| i as f64).collect()),
    )
    .unwrap();
    let mut best = u128::MAX;
    for _ in 0..6 {
        let t = std::time::Instant::now();
        std::hint::black_box(s.to_period(freq).unwrap());
        let e = t.elapsed().as_nanos();
        if e < best {
            best = e;
        }
    }
    println!("to_period_{freq} n={n}: best={best}ns");
}
