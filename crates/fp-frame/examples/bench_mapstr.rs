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
    for _ in 0..6 {
        let t = std::time::Instant::now();
        f();
        b = b.min(t.elapsed().as_nanos());
    }
    println!("{l}: {:.2}ms", b as f64 / 1e6);
}
fn main() {
    let n = 1_000_000usize;
    let ncat = 1000usize;
    // Utf8 input column: "cat_<k>" for k in 0..ncat
    let v: Vec<Scalar> = (0..n)
        .map(|i| Scalar::Utf8(format!("cat_{}", sm(i, 1) % ncat as u64)))
        .collect();
    let idx = Index::from_range(0, n as i64, 1);
    let s = Series::new("s", idx, Column::from_values(v).unwrap()).unwrap();
    // mapping "cat_k" -> f64 k  (Utf8->Float64, full coverage)
    let map_f64: Vec<(Scalar, Scalar)> = (0..ncat)
        .map(|k| (Scalar::Utf8(format!("cat_{k}")), Scalar::Float64(k as f64)))
        .collect();
    // mapping "cat_k" -> "grp_<k%10>" (Utf8->Utf8 recode, full coverage)
    let map_str: Vec<(Scalar, Scalar)> = (0..ncat)
        .map(|k| {
            (
                Scalar::Utf8(format!("cat_{k}")),
                Scalar::Utf8(format!("grp_{}", k % 10)),
            )
        })
        .collect();
    timeit("s.map Utf8->Float64 (1M, 1000 cats)", || {
        std::hint::black_box(s.map(&map_f64).unwrap());
    });
    timeit("s.map Utf8->Utf8 recode (1M, 1000 cats)", || {
        std::hint::black_box(s.map(&map_str).unwrap());
    });
}
