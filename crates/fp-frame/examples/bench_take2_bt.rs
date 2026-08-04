use fp_columnar::Column;
use fp_frame::Series;
use fp_index::Index;
use fp_types::Scalar;
fn sm(i: usize, s: u64) -> u64 {
    let mut h = (i as u64).wrapping_add(s).wrapping_mul(0x9E3779B97F4A7C15);
    h = (h ^ (h >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    h ^ (h >> 31)
}
fn t(l: &str, mut f: impl FnMut()) {
    let mut b = u128::MAX;
    for _ in 0..6 {
        let x = std::time::Instant::now();
        f();
        b = b.min(x.elapsed().as_nanos());
    }
    println!("{l}: {:.2}ms", b as f64 / 1e6);
}
fn dig(s: &Series) -> u64 {
    let mut h = 1469598103934665603u64;
    for l in s.index().labels().iter() {
        let b = format!("{:?}", l);
        for by in b.bytes() {
            h ^= by as u64;
            h = h.wrapping_mul(1099511628211);
        }
    }
    for v in s.column().values().iter() {
        let b = format!("{:?}", v);
        for by in b.bytes() {
            h ^= by as u64;
            h = h.wrapping_mul(1099511628211);
        }
    }
    h
}
fn main() {
    let n = 2_000_000usize;
    let idx = Index::from_range(0, n as i64, 1);
    let v = Series::new(
        "v",
        idx.clone(),
        Column::from_values((0..n).map(|i| Scalar::Float64(i as f64)).collect()).unwrap(),
    )
    .unwrap();
    // include some negative indices to exercise the negative branch
    let mut pos: Vec<i64> = (0..n)
        .map(|i| {
            let p = (sm(i, 3) % n as u64) as i64;
            if i % 5 == 0 { p - (n as i64) } else { p }
        })
        .collect();
    println!("digest_random={}", dig(&v.take(&pos).unwrap()));
    t("take_f64_random", || {
        std::hint::black_box(v.take(&pos).unwrap());
    });
    pos.sort_unstable();
    println!("digest_sorted={}", dig(&v.take(&pos).unwrap()));
    t("take_f64_sorted", || {
        std::hint::black_box(v.take(&pos).unwrap());
    });
}
