//! SeriesGroupBy by a nullable i64 key (10% missing). bench_gbnull <n> <card>
use fp_columnar::Column;
use fp_frame::Series;
use fp_index::{Index, IndexLabel};
use fp_types::Scalar;
fn sm(i: usize, s: u64) -> u64 {
    let mut h = (i as u64).wrapping_add(s).wrapping_mul(0x9E3779B97F4A7C15);
    h = (h ^ (h >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    h ^ (h >> 31)
}
fn timeit<F: FnMut()>(l: &str, mut f: F) {
    let mut b = u128::MAX;
    for _ in 0..6 {
        let t = std::time::Instant::now();
        f();
        b = b.min(t.elapsed().as_nanos());
    }
    println!("{l}: {:.2}ms", b as f64 / 1e6);
}
fn main() {
    let n: usize = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(2_000_000);
    let card: usize = std::env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(1000);
    let labels: Vec<IndexLabel> = (0..n as i64).map(IndexLabel::Int64).collect();
    // nullable i64 key: 10% missing
    let kv: Vec<Scalar> = (0..n)
        .map(|i| {
            if sm(i, 0).is_multiple_of(10) {
                Scalar::Null(fp_types::NullKind::Null)
            } else {
                Scalar::Int64((sm(i, 0) % card as u64) as i64)
            }
        })
        .collect();
    let key = Series::from_values("k", labels.clone(), kv).unwrap();
    let val = Series::new(
        "v",
        Index::new(labels),
        Column::from_f64_values((0..n).map(|i| (sm(i, 7) % 100000) as f64).collect()),
    )
    .unwrap();
    timeit("sum", || {
        std::hint::black_box(val.groupby(&key).unwrap().sum().unwrap().len());
    });
    timeit("mean", || {
        std::hint::black_box(val.groupby(&key).unwrap().mean().unwrap().len());
    });
    timeit("count", || {
        std::hint::black_box(val.groupby(&key).unwrap().count().unwrap().len());
    });
}
