use fp_columnar::Column;
use fp_frame::Series;
use fp_index::Index;
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
fn main() {
    let n: usize = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(2_000_000);
    let cats = [
        "red", "green", "blue", "yellow", "orange", "purple", "black", "white",
    ];
    let uvals: Vec<Scalar> = (0..n)
        .map(|i| Scalar::Utf8(cats[i % cats.len()].to_string()))
        .collect();
    let su = Series::new(
        "u",
        Index::from_range(0, n as i64, 1),
        Column::from_values(uvals).unwrap(),
    )
    .unwrap();
    let iv: Vec<i64> = (0..n as i64).map(|i| i % 1000).collect();
    let si = Series::new(
        "i",
        Index::from_range(0, n as i64, 1),
        Column::from_i64_values(iv),
    )
    .unwrap();
    timeit("factorize_utf8", || {
        let r = su.factorize().unwrap();
        std::hint::black_box(r.0.len());
    });
    timeit("factorize_i64", || {
        let r = si.factorize().unwrap();
        std::hint::black_box(r.0.len());
    });
    timeit("get_dummies_utf8", || {
        std::hint::black_box(su.str().get_dummies(",").unwrap().shape());
    });
    timeit("value_counts_utf8", || {
        std::hint::black_box(su.value_counts().unwrap().len());
    });
}
