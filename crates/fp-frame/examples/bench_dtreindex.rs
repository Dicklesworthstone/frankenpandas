use fp_columnar::Column;
use fp_frame::Series;
use fp_index::{Index, IndexLabel};
fn timeit(l: &str, mut f: impl FnMut()) {
    let mut b = u128::MAX;
    for _ in 0..6 {
        let t = std::time::Instant::now();
        f();
        b = b.min(t.elapsed().as_nanos());
    }
    println!("{l}: {:.2}ms", b as f64 / 1e6);
}
fn main() {
    let n = 1_000_000usize;
    let base = 1_600_000_000_000_000_000i64;
    // datetime series indexed 0..n; reindex to [n/2 .. n/2+n) -> half overlap, half null-fill
    let dt = Series::new(
        "dt",
        Index::from_range(0, n as i64, 1),
        Column::from_datetime64_values((0..n).map(|i| base + i as i64 * 1_000_000_000).collect()),
    )
    .unwrap();
    let i64s = Series::new(
        "i",
        Index::from_range(0, n as i64, 1),
        Column::from_i64_values_owned((0..n).map(|i| i as i64).collect()),
    )
    .unwrap();
    let new_labels: Vec<IndexLabel> = ((n / 2) as i64..(n / 2 + n) as i64)
        .map(IndexLabel::Int64)
        .collect();
    timeit("datetime reindex 1M half-miss", || {
        std::hint::black_box(dt.reindex(new_labels.clone()).unwrap());
    });
    timeit("i64 reindex 1M half-miss", || {
        std::hint::black_box(i64s.reindex(new_labels.clone()).unwrap());
    });
}
