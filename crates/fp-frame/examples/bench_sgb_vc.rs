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
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(1_000_000);
    let g: i64 = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(1000);
    let it: usize = a.get(3).and_then(|s| s.parse().ok()).unwrap_or(6);
    let by = Series::new(
        "k",
        Index::new_known_unique_int64_unit_range(0, n),
        Column::from_i64_values((0..n).map(|i| (i as i64) % g).collect()),
    )
    .unwrap();
    let v = Series::new(
        "v",
        Index::new_known_unique_int64_unit_range(0, n),
        Column::from_i64_values((0..n).map(|i| (sm(i, 1) % 50) as i64).collect()),
    )
    .unwrap();
    let mut best = u128::MAX;
    for _ in 0..it {
        let t = std::time::Instant::now();
        std::hint::black_box(v.groupby(&by).unwrap().value_counts().unwrap());
        let e = t.elapsed().as_nanos();
        if e < best {
            best = e;
        }
    }
    println!("sgb_value_counts n={n} g={g}: best={best}ns");
}
