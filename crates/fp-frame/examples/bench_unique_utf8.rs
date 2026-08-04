//! Series.unique() + nunique() on a Scalar-backed Utf8 series. Run: -- 2000000 5000
use std::time::Instant;

use fp_frame::Series;
use fp_index::IndexLabel;
use fp_types::Scalar;
fn best<F: FnMut()>(it: usize, mut f: F) -> u128 {
    let mut b = u128::MAX;
    for _ in 0..it {
        let t = Instant::now();
        f();
        let e = t.elapsed().as_nanos();
        if e < b {
            b = e;
        }
    }
    b
}
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(2_000_000);
    let card: i64 = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(5000);
    let it: usize = a.get(3).and_then(|s| s.parse().ok()).unwrap_or(12);
    let labels: Vec<IndexLabel> = (0..n as i64).map(IndexLabel::Int64).collect();
    let s = Series::from_values(
        "c",
        labels,
        (0..n)
            .map(|i| {
                Scalar::Utf8(format!(
                    "v{:08}",
                    ((i as i64).wrapping_mul(2654435761) >> 13) % card
                ))
            })
            .collect(),
    )
    .unwrap();
    let u = best(it, || {
        std::hint::black_box(s.unique());
    });
    let nu = best(it, || {
        std::hint::black_box(s.nunique());
    });
    println!("unique_utf8 n={n} card={card}: unique={u}ns nunique={nu}ns");
}
