//! Series.sort_values() on a Scalar-backed Utf8 series. Run: -- 2000000
use std::time::Instant;

use fp_frame::Series;
use fp_index::IndexLabel;
use fp_types::Scalar;
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(2_000_000);
    let it: usize = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(10);
    let labels: Vec<IndexLabel> = (0..n as i64).map(IndexLabel::Int64).collect();
    let s = Series::from_values(
        "c",
        labels,
        (0..n)
            .map(|i| {
                Scalar::Utf8(format!(
                    "k{:010}",
                    ((i as i64).wrapping_mul(2654435761) >> 7) & 0x3FFFFFFF
                ))
            })
            .collect(),
    )
    .unwrap();
    let mut best = u128::MAX;
    for _ in 0..it {
        let t = Instant::now();
        std::hint::black_box(s.sort_values(true).unwrap());
        let e = t.elapsed().as_nanos();
        if e < best {
            best = e;
        }
    }
    println!("sort_utf8 n={n}: best={best}ns");
}
