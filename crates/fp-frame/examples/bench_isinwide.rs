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
    // wide-i64 haystack
    let hv: Vec<Scalar> = (0..n)
        .map(|i| Scalar::Int64(((sm(i, 1) % (n as u64)) as i64) * 2654435761))
        .collect();
    let s = Series::new("s", idx.clone(), Column::from_values(hv).unwrap()).unwrap();
    // needle set of ~100k wide values
    let needles: Vec<Scalar> = (0..100_000usize)
        .map(|i| Scalar::Int64(((sm(i, 3) % (n as u64)) as i64) * 2654435761))
        .collect();
    timeit("isin wide-i64 (100k needles)", || {
        std::hint::black_box(s.isin(&needles).unwrap());
    });
    // searchsorted many needles into sorted wide-i64
    let mut sorted: Vec<i64> = (0..n)
        .map(|i| ((sm(i, 1) % (n as u64)) as i64) * 2654435761)
        .collect();
    sorted.sort_unstable();
    sorted.dedup();
    let sv: Vec<Scalar> = sorted.iter().map(|&v| Scalar::Int64(v)).collect();
    let ss = Series::new(
        "ss",
        Index::from_range(0, sv.len() as i64, 1),
        Column::from_values(sv).unwrap(),
    )
    .unwrap();
    let nd: Vec<Scalar> = (0..500_000usize)
        .map(|i| Scalar::Int64(((sm(i, 5) % (n as u64)) as i64) * 2654435761))
        .collect();
    timeit("searchsorted wide-i64 (500k needles)", || {
        std::hint::black_box(ss.searchsorted_values(&nd, "left").unwrap());
    });
}
