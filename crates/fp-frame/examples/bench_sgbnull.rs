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
    let g: i64 = 1000;
    let by = Series::new(
        "k",
        Index::new_known_unique_int64_unit_range(0, n),
        Column::from_i64_values((0..n).map(|i| (sm(i, 0) as i64).rem_euclid(g)).collect()),
    )
    .unwrap();
    let av: Vec<Scalar> = (0..n)
        .map(|i| {
            if sm(i, 1).is_multiple_of(5) {
                Scalar::Null(NullKind::NaN)
            } else {
                Scalar::Float64(sm(i, 1) as f64)
            }
        })
        .collect();
    let v = Series::new(
        "v",
        Index::new_known_unique_int64_unit_range(0, n),
        Column::from_values(av).unwrap(),
    )
    .unwrap();
    let mut best = u128::MAX;
    for _ in 0..6 {
        let t = std::time::Instant::now();
        let r = match op {
            "sum" => v.groupby(&by).unwrap().sum(),
            "mean" => v.groupby(&by).unwrap().mean(),
            "min" => v.groupby(&by).unwrap().min(),
            "std" => v.groupby(&by).unwrap().std(),
            "var" => v.groupby(&by).unwrap().var(),
            "median" => v.groupby(&by).unwrap().median(),
            _ => panic!(),
        };
        std::hint::black_box(r.unwrap());
        let e = t.elapsed().as_nanos();
        if e < best {
            best = e;
        }
    }
    println!("sgbnull_{op} n={n} g={g}: best={best}ns");
}
