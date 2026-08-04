use fp_columnar::Column;
use fp_frame::Series;
use fp_index::Index;
fn sm(i: usize, s: u64) -> u64 {
    let mut h = (i as u64).wrapping_add(s).wrapping_mul(0x9E3779B97F4A7C15);
    h = (h ^ (h >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    h ^= h >> 31;
    h
}
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let op = a.get(1).map(String::as_str).unwrap_or("cumsum");
    let n: usize = 1_000_000;
    let data: Vec<f64> = (0..n).map(|i| (sm(i, 1) % 100_000) as f64).collect();
    let s = Series::new(
        "v",
        Index::new_known_unique_int64_unit_range(0, n),
        Column::from_f64_values(data),
    )
    .unwrap();
    let mut best = u128::MAX;
    for _ in 0..6 {
        let t = std::time::Instant::now();
        match op {
            "cumsum" => {
                std::hint::black_box(s.cumsum().unwrap());
            }
            "cummax" => {
                std::hint::black_box(s.cummax().unwrap());
            }
            "diff" => {
                std::hint::black_box(s.diff(1).unwrap());
            }
            "pct_change" => {
                std::hint::black_box(s.pct_change(1).unwrap());
            }
            "rank" => {
                std::hint::black_box(s.rank("average", true, "keep").unwrap());
            }
            "nlargest" => {
                std::hint::black_box(s.nlargest(10).unwrap());
            }
            "mode" => {
                std::hint::black_box(s.mode().unwrap());
            }
            _ => panic!(),
        }
        let e = t.elapsed().as_nanos();
        if e < best {
            best = e;
        }
    }
    println!("probe_{op} n={n}: best={best}ns");
}
