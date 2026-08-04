//! Series.str.contains(literal) on a Scalar-backed Utf8 series. Run: -- 2000000
use std::time::Instant;

use fp_frame::Series;
use fp_index::IndexLabel;
use fp_types::Scalar;
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(2_000_000);
    let it: usize = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(12);
    let labels: Vec<IndexLabel> = (0..n as i64).map(IndexLabel::Int64).collect();
    let s = Series::from_values(
        "c",
        labels,
        (0..n)
            .map(|i| Scalar::Utf8(format!("item_{:08}_xyz", i)))
            .collect(),
    )
    .unwrap();
    // op: "simple" = str.contains (apply_str_bool); "regex"/"literal" =
    // contains_with_options (str_boolean_with_na, pandas default regex=True path).
    let op = a.get(3).map(String::as_str).unwrap_or("regex");
    let mut best = u128::MAX;
    for _ in 0..it {
        let t = Instant::now();
        let r = match op {
            "simple" => s.str().contains("777"),
            "regex" => s.str().contains_with_options("777", true, None, true),
            "literal" => s.str().contains_with_options("777", true, None, false),
            _ => panic!(),
        };
        std::hint::black_box(r.unwrap());
        let e = t.elapsed().as_nanos();
        if e < best {
            best = e;
        }
    }
    println!("str_contains_{op} n={n}: best={best}ns");
}
