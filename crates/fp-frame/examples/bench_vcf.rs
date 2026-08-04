use fp_columnar::Column;
use fp_frame::Series;
use fp_index::Index;
use fp_types::Scalar;
fn sm(i: usize, s: u64) -> u64 {
    let mut h = (i as u64).wrapping_add(s).wrapping_mul(0x9E3779B97F4A7C15);
    h = (h ^ (h >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    h ^ (h >> 31)
}
fn timeit(l: &str, mut f: impl FnMut()) {
    let mut b = u128::MAX;
    for _ in 0..5 {
        let t = std::time::Instant::now();
        f();
        b = b.min(t.elapsed().as_nanos());
    }
    println!("{l}: {:.2}ms", b as f64 / 1e6);
}
fn main() {
    let n = 2_000_000usize;
    let idx = Index::from_range(0, n as i64, 1);
    // wide high-card f64 (~half distinct)
    let fv: Vec<Scalar> = (0..n)
        .map(|i| Scalar::Float64(((sm(i, 1) % (n as u64 / 2)) as f64) * 1.5))
        .collect();
    let sf = Series::new("sf", idx.clone(), Column::from_values(fv).unwrap()).unwrap();
    timeit("value_counts WIDE-f64", || {
        std::hint::black_box(sf.value_counts().unwrap());
    });
    // wide high-card Utf8 (~half distinct)
    let uv: Vec<Scalar> = (0..n)
        .map(|i| Scalar::Utf8(format!("id{}", sm(i, 1) % (n as u64 / 2))))
        .collect();
    let su = Series::new("su", idx.clone(), Column::from_values(uv).unwrap()).unwrap();
    timeit("value_counts WIDE-utf8", || {
        std::hint::black_box(su.value_counts().unwrap());
    });
}
