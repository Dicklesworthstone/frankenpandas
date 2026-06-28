//! Column::nunique / duplicated over a high-card 5M Int64 column. bench_hashops <n> <op>
use fp_columnar::Column;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(5_000_000);
    let op = a.get(2).map(String::as_str).unwrap_or("nunique");
    let mode = a.get(3).map(String::as_str).unwrap_or("dense");
    let data: Vec<i64> = if mode == "wide" {
        // Sparse full-range i64, ~5M distinct: no dense bitset possible -> hashset.
        (0..n as i64).map(|i| i.wrapping_mul(2_654_435_761)).collect()
    } else {
        // ~1M distinct in [0,1M): dense direct-address regime.
        (0..n as i64).map(|i| (i.wrapping_mul(2_654_435_761)).rem_euclid(1_000_000)).collect()
    };
    let col = Column::from_i64_values(data);
    let mut best = u128::MAX;
    for _ in 0..6 {
        let t = std::time::Instant::now();
        match op {
            "duplicated" => {
                let r = col.duplicated().unwrap();
                std::hint::black_box(r.len());
            }
            _ => {
                let r = col.nunique();
                std::hint::black_box(&r);
            }
        }
        let e = t.elapsed().as_nanos();
        if e < best {
            best = e;
        }
    }
    println!("{op} i64 n={n}: best={best}ns ({:.2}ms)", best as f64 / 1e6);
}
