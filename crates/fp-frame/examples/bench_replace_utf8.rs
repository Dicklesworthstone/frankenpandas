//! Series.replace(Utf8->Utf8, few keys) on a Utf8 series. Run: -- 2000000 20 5
use std::time::Instant;

use fp_frame::Series;
use fp_index::IndexLabel;
use fp_types::Scalar;
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(2_000_000);
    let card: i64 = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(20);
    let nrepl: i64 = a.get(3).and_then(|s| s.parse().ok()).unwrap_or(5);
    let it: usize = a.get(4).and_then(|s| s.parse().ok()).unwrap_or(12);
    let labels: Vec<IndexLabel> = (0..n as i64).map(IndexLabel::Int64).collect();
    let s = Series::from_values(
        "c",
        labels,
        (0..n)
            .map(|i| {
                Scalar::Utf8(format!(
                    "cat{:03}",
                    ((i as i64).wrapping_mul(2654435761) >> 13) % card
                ))
            })
            .collect(),
    )
    .unwrap();
    let repl: Vec<(Scalar, Scalar)> = (0..nrepl)
        .map(|k| {
            (
                Scalar::Utf8(format!("cat{:03}", k)),
                Scalar::Utf8(format!("NEW{}", k)),
            )
        })
        .collect();
    let mut best = u128::MAX;
    for _ in 0..it {
        let t = Instant::now();
        std::hint::black_box(s.replace(&repl).unwrap());
        let e = t.elapsed().as_nanos();
        if e < best {
            best = e;
        }
    }
    println!("replace_utf8 n={n} card={card} nrepl={nrepl}: best={best}ns");
}
