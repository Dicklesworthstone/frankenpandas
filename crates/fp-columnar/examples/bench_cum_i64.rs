//! Column cumsum/cummax over a 5M Int64 column. bench_cum_i64 <n> <op>
use fp_columnar::Column;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(5_000_000);
    let op = a.get(2).map(String::as_str).unwrap_or("cumsum");
    let col = Column::from_i64_values((0..n as i64).map(|i| i % 1000).collect());
    let mut best = u128::MAX;
    for _ in 0..6 {
        let t = std::time::Instant::now();
        let r = match op {
            "cummax" => col.cummax().unwrap(),
            "cumprod" => col.cumprod().unwrap(),
            "cummin" => col.cummin().unwrap(),
            _ => col.cumsum().unwrap(),
        };
        std::hint::black_box(r.len());
        let e = t.elapsed().as_nanos();
        if e < best {
            best = e;
        }
    }
    println!("{op} i64 n={n}: best={best}ns ({:.2}ms)", best as f64 / 1e6);
}
