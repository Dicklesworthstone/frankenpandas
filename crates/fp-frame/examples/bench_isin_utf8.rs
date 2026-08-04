//! Series.isin(set) on a Scalar-backed Utf8 series. Run: -- 2000000 50 1000
use std::time::Instant;

use fp_frame::Series;
use fp_index::IndexLabel;
use fp_types::Scalar;
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(2_000_000);
    let card: i64 = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(50);
    let setsz: i64 = a.get(3).and_then(|s| s.parse().ok()).unwrap_or(1000);
    let it: usize = a.get(4).and_then(|s| s.parse().ok()).unwrap_or(12);
    let labels: Vec<IndexLabel> = (0..n as i64).map(IndexLabel::Int64).collect();
    let s = Series::from_values(
        "c",
        labels,
        (0..n)
            .map(|i| {
                Scalar::Utf8(format!(
                    "v{:06}",
                    ((i as i64).wrapping_mul(2654435761) >> 13) % card
                ))
            })
            .collect(),
    )
    .unwrap();
    let needles: Vec<Scalar> = (0..setsz)
        .map(|k| Scalar::Utf8(format!("v{:06}", k)))
        .collect();
    let mut best = u128::MAX;
    for _ in 0..it {
        let t = Instant::now();
        std::hint::black_box(s.isin(&needles).unwrap());
        let e = t.elapsed().as_nanos();
        if e < best {
            best = e;
        }
    }
    println!("isin_utf8 n={n} card={card} setsz={setsz}: best={best}ns");
}
