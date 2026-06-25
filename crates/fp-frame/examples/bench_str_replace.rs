use fp_columnar::Column;
use fp_frame::Series;
use fp_index::Index;
use fp_types::Scalar;
fn sm(i: usize, s: u64) -> u64 {
    let mut h = (i as u64).wrapping_add(s).wrapping_mul(0x9E3779B97F4A7C15);
    h = (h ^ (h >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    h ^= h >> 31;
    h
}
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let op = a.get(1).map(String::as_str).unwrap_or("replace");
    let n: usize = 1_000_000;
    let vals: Vec<Scalar> = (0..n)
        .map(|i| Scalar::Utf8(format!("item_{:08}_xyz", sm(i, 1) % 1_000_000)))
        .collect();
    let s = Series::new(
        "v",
        Index::new_known_unique_int64_unit_range(0, n),
        Column::from_values(vals).unwrap(),
    )
    .unwrap();
    let mut best = u128::MAX;
    for _ in 0..6 {
        let t = std::time::Instant::now();
        let r = match op {
            "replace" => s.str().replace("item", "ROW"),
            "repeat" => s.str().repeat(2),
            "capitalize" => s.str().capitalize(),
            "title" => s.str().title(),
            "casefold" => s.str().casefold(),
            _ => panic!(),
        };
        std::hint::black_box(r.unwrap());
        let e = t.elapsed().as_nanos();
        if e < best {
            best = e;
        }
    }
    println!("str_{op} n={n}: best={best}ns");
}
