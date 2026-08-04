//! Column::nlargest(k) over 5M f64/i64. bench_nlargest <n> <k> <dtype>
use fp_columnar::Column;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(5_000_000);
    let k: usize = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(10);
    let dt = a.get(3).map(String::as_str).unwrap_or("f64");
    let col = if dt == "dt" {
        Column::from_datetime64_values(
            (0..n as i64)
                .map(|i| i.wrapping_mul(2_654_435_761))
                .collect(),
        )
    } else if dt == "i64" {
        Column::from_i64_values(
            (0..n as i64)
                .map(|i| i.wrapping_mul(2_654_435_761))
                .collect(),
        )
    } else {
        Column::from_f64_values(
            (0..n as u64)
                .map(|i| {
                    let mut z = i.wrapping_mul(0x9E37_79B9_7F4A_7C15);
                    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
                    (z >> 11) as f64 / (1u64 << 53) as f64
                })
                .collect(),
        )
    };
    let mut best = u128::MAX;
    for _ in 0..6 {
        let t = std::time::Instant::now();
        let r = col.nlargest(k).unwrap();
        std::hint::black_box(r.len());
        let e = t.elapsed().as_nanos();
        if e < best {
            best = e;
        }
    }
    println!(
        "nlargest({k}) {dt} n={n}: best={best}ns ({:.2}ms)",
        best as f64 / 1e6
    );
}
