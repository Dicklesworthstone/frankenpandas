use fp_columnar::Column;
use fp_frame::Series;
use fp_index::{Index, IndexLabel};
use fp_types::Scalar;
fn sm(i: usize, s: u64) -> u64 {
    let mut h = (i as u64).wrapping_add(s).wrapping_mul(0x9E3779B97F4A7C15);
    h = (h ^ (h >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    h ^ (h >> 31)
}
fn timeit(l: &str, mut f: impl FnMut()) {
    let mut b = u128::MAX;
    for _ in 0..8 {
        let t = std::time::Instant::now();
        f();
        b = b.min(t.elapsed().as_nanos());
    }
    println!("{l}: {:.2}ms", b as f64 / 1e6);
}
fn main() {
    let n = 2_000_000usize;
    let b = Series::new(
        "b",
        Index::from_range(0, n as i64, 1),
        Column::from_bool_values((0..n).map(|i| sm(i, 1).is_multiple_of(2)).collect()),
    )
    .unwrap();
    let nl: Vec<IndexLabel> = ((n / 2) as i64..(n / 2 + n) as i64)
        .map(IndexLabel::Int64)
        .collect();
    // correctness: first entries present, tail (>=n) null
    let r = b.reindex(nl.clone()).unwrap();
    let vals = r.column().values();
    let ok = matches!(&vals[0], Scalar::Bool(x) if *x==sm(n/2,1).is_multiple_of(2))
        && matches!(&vals[n - 1], Scalar::Null(_));
    println!("bool reindex correct: {ok}");
    timeit("bool reindex 2M half-miss", || {
        std::hint::black_box(b.reindex(nl.clone()).unwrap());
    });
}
