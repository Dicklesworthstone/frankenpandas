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
    let fv: Vec<Scalar> = (0..n)
        .map(|i| Scalar::Float64((sm(i, 9) % 100000) as f64 * 0.5))
        .collect();
    let s = Series::new("s", idx.clone(), Column::from_values(fv).unwrap()).unwrap();
    timeit("nlargest(100)", || {
        std::hint::black_box(s.nlargest(100).unwrap());
    });
    timeit("nsmallest(100)", || {
        std::hint::black_box(s.nsmallest(100).unwrap());
    });
    timeit("clip(100,90000)", || {
        std::hint::black_box(s.clip(Some(100.0), Some(90000.0)).unwrap());
    });
    timeit("rolling(100).mean", || {
        std::hint::black_box(s.rolling(100, None).mean().unwrap());
    });
    timeit("rolling(100).sum", || {
        std::hint::black_box(s.rolling(100, None).sum().unwrap());
    });
    // with nulls for fillna/interpolate
    let nv: Vec<Scalar> = (0..n)
        .map(|i| {
            if sm(i, 3).is_multiple_of(4) {
                Scalar::Null(fp_types::NullKind::Null)
            } else {
                Scalar::Float64((sm(i, 9) % 1000) as f64)
            }
        })
        .collect();
    let sn = Series::new("sn", idx.clone(), Column::from_values(nv).unwrap()).unwrap();
    timeit("fillna(0)", || {
        std::hint::black_box(sn.fillna(&Scalar::Float64(0.0)).unwrap());
    });
    timeit("interpolate", || {
        std::hint::black_box(sn.interpolate().unwrap());
    });
}
