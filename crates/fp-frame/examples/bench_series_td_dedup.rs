//! Series value_counts/nunique/unique/duplicated/drop_duplicates over a Timedelta64 VALUE
//! column @200k. Run: bench_series_td_dedup <n> <op>
use fp_columnar::Column;
use fp_frame::Series;
use fp_index::{Index, IndexLabel};
use fp_types::{DType, Scalar};

fn sm(i: usize, s: u64) -> u64 {
    let mut h = (i as u64).wrapping_add(s).wrapping_mul(0x9E3779B97F4A7C15);
    h = (h ^ (h >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    h ^= h >> 31;
    h
}
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(200_000);
    let op = a.get(2).map(String::as_str).unwrap_or("value_counts");
    let card = (n / 4).max(1) as u64;
    let vals: Vec<Scalar> = (0..n)
        .map(|i| Scalar::Timedelta64((sm(i, 0) % card) as i64 * 1_000_000_000))
        .collect();
    let labels: Vec<IndexLabel> = (0..n as i64).map(IndexLabel::Int64).collect();
    let s = Series::new(
        "s",
        Index::new(labels),
        Column::new(DType::Timedelta64, vals).unwrap(),
    )
    .unwrap();
    let mut best = u128::MAX;
    for _ in 0..6 {
        let t = std::time::Instant::now();
        match op {
            "value_counts" => {
                std::hint::black_box(s.value_counts().unwrap());
            }
            "nunique" => {
                std::hint::black_box(s.nunique());
            }
            "unique" => {
                std::hint::black_box(s.unique());
            }
            "duplicated" => {
                std::hint::black_box(s.duplicated().unwrap());
            }
            "drop_duplicates" => {
                std::hint::black_box(s.drop_duplicates().unwrap());
            }
            _ => panic!("op"),
        }
        let e = t.elapsed().as_nanos();
        if e < best {
            best = e;
        }
    }
    println!("series_td_{op} n={n}: best={best}ns");
}
