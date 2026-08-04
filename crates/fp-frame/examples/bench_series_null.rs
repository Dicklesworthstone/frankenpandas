use fp_columnar::Column;
use fp_frame::Series;
use fp_index::Index;
use fp_types::{NullKind, Scalar};
fn sm(i: usize, s: u64) -> u64 {
    let mut h = (i as u64).wrapping_add(s).wrapping_mul(0x9E3779B97F4A7C15);
    h = (h ^ (h >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    h ^= h >> 31;
    h
}
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let op = a.get(1).map(String::as_str).unwrap_or("sum");
    let n: usize = 1_000_000;
    let av: Vec<Scalar> = (0..n)
        .map(|i| {
            if sm(i, 1).is_multiple_of(5) {
                Scalar::Null(NullKind::NaN)
            } else {
                Scalar::Float64((sm(i, 1) % 100_000) as f64)
            }
        })
        .collect();
    let s = Series::new(
        "v",
        Index::new_known_unique_int64_unit_range(0, n),
        Column::from_values(av).unwrap(),
    )
    .unwrap();
    let mut best = u128::MAX;
    for _ in 0..8 {
        let t = std::time::Instant::now();
        match op {
            "sum" => {
                std::hint::black_box(s.sum().unwrap());
            }
            "mean" => {
                std::hint::black_box(s.mean().unwrap());
            }
            "std" => {
                std::hint::black_box(s.std().unwrap());
            }
            "var" => {
                std::hint::black_box(s.var().unwrap());
            }
            _ => panic!(),
        }
        let e = t.elapsed().as_nanos();
        if e < best {
            best = e;
        }
    }
    println!("series_null_{op} n={n}: best={best}ns");
}
