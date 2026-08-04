//! Scalar-backed Utf8 op survey (from_values construction). bench_u8survey <n> <card>
use fp_columnar::Column;
use fp_frame::Series;
use fp_index::{Index, IndexLabel};
use fp_types::Scalar;
fn timeit<F: FnMut()>(label: &str, mut f: F) {
    let mut best = u128::MAX;
    for _ in 0..6 {
        let t = std::time::Instant::now();
        f();
        best = best.min(t.elapsed().as_nanos());
    }
    println!("{label}: {:.2}ms", best as f64 / 1e6);
}
fn sm(i: usize, s: u64) -> u64 {
    let mut h = (i as u64).wrapping_add(s).wrapping_mul(0x9E3779B97F4A7C15);
    h = (h ^ (h >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    h ^ (h >> 31)
}
fn main() {
    let n: usize = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(2_000_000);
    let card: usize = std::env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(8);
    let cats: Vec<String> = (0..card)
        .map(|c| format!("category_label_{c:04}"))
        .collect();
    let vals: Vec<Scalar> = (0..n)
        .map(|i| Scalar::Utf8(cats[(sm(i, 0) as usize) % card].clone()))
        .collect();
    let labels: Vec<IndexLabel> = (0..n as i64).map(IndexLabel::Int64).collect();
    let s = Series::from_values("s", labels, vals).unwrap();
    // i64 key for groupby-by
    let key = Series::new(
        "k",
        Index::from_range(0, n as i64, 1),
        Column::from_i64_values(
            (0..n as i64)
                .map(|i| (sm(i as usize, 9) % card as u64) as i64)
                .collect(),
        ),
    )
    .unwrap();
    timeit("value_counts", || {
        std::hint::black_box(s.value_counts().unwrap().len());
    });
    timeit("unique", || {
        std::hint::black_box(s.unique().len());
    });
    timeit("nunique", || {
        std::hint::black_box(s.nunique());
    });
    timeit("duplicated", || {
        std::hint::black_box(s.duplicated().unwrap().len());
    });
    timeit("sort_values", || {
        std::hint::black_box(s.sort_values(true).unwrap().len());
    });
    timeit("groupby_count", || {
        std::hint::black_box(s.groupby(&key).unwrap().count().unwrap().len());
    });
    timeit("isin", || {
        std::hint::black_box(
            s.isin(&[Scalar::Utf8(cats[0].clone()), Scalar::Utf8(cats[1].clone())])
                .unwrap()
                .len(),
        );
    });
}
