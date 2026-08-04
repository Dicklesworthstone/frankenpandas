//! Column::replace scalar over 5M i64. bench_replace <n>
use fp_columnar::Column;
use fp_types::Scalar;
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(5_000_000);
    let col = Column::from_i64_values((0..n as i64).map(|i| i % 1000).collect());
    let to_rep = vec![Scalar::Int64(5)];
    let rep = vec![Scalar::Int64(-1)];
    let mut best = u128::MAX;
    for _ in 0..6 {
        let t = std::time::Instant::now();
        let r = col.replace(&to_rep, &rep).unwrap();
        std::hint::black_box(r.len());
        let e = t.elapsed().as_nanos();
        if e < best {
            best = e;
        }
    }
    println!(
        "replace i64 n={n}: best={best}ns ({:.2}ms)",
        best as f64 / 1e6
    );
}
