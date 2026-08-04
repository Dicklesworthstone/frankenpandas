//! Series.dt.total_seconds() @5M (Timedelta64). bench_tdsec <n>
use fp_frame::Series;
use fp_index::IndexLabel;
use fp_types::Scalar;
fn main() {
    let n: usize = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(5_000_000);
    let vals: Vec<Scalar> = (0..n)
        .map(|i| Scalar::Timedelta64(((i as i64).wrapping_mul(1_000_003)) % 86_400_000_000_000))
        .collect();
    let labels: Vec<IndexLabel> = (0..n as i64).map(IndexLabel::Int64).collect();
    let s = Series::from_values("td", labels, vals).unwrap();
    let mut best = u128::MAX;
    for _ in 0..6 {
        let t = std::time::Instant::now();
        std::hint::black_box(s.dt().total_seconds().unwrap().len());
        let e = t.elapsed().as_nanos();
        if e < best {
            best = e;
        }
    }
    println!("total_seconds n={n}: best={:.2}ms", best as f64 / 1e6);
}
