use fp_columnar::Column;
use fp_frame::Series;
use fp_index::Index;
use fp_types::{NullKind, Scalar};
fn sm(i: usize, s: u64) -> u64 {
    let mut h = (i as u64).wrapping_add(s).wrapping_mul(0x9E3779B97F4A7C15);
    h = (h ^ (h >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    h ^ (h >> 31)
}
fn t(l: &str, mut f: impl FnMut()) {
    let mut b = u128::MAX;
    for _ in 0..4 {
        let x = std::time::Instant::now();
        f();
        b = b.min(x.elapsed().as_nanos());
    }
    println!("{l}: {:.2}ms", b as f64 / 1e6);
}
fn main() {
    let n = 2_000_000usize;
    let idx = Index::from_range(0, n as i64, 1);
    let v = Series::new(
        "v",
        idx.clone(),
        Column::from_values(
            (0..n)
                .map(|i| Scalar::Float64((sm(i, 7) % 1000000) as f64))
                .collect(),
        )
        .unwrap(),
    )
    .unwrap();
    let vn = Series::new(
        "vn",
        idx.clone(),
        Column::from_values(
            (0..n)
                .map(|i| {
                    if sm(i, 1).is_multiple_of(10) {
                        Scalar::Null(NullKind::NaN)
                    } else {
                        Scalar::Float64((sm(i, 7) % 1000000) as f64)
                    }
                })
                .collect(),
        )
        .unwrap(),
    )
    .unwrap();
    let qs: Vec<f64> = (1..100).map(|i| i as f64 / 100.0).collect();
    t("qlist99_allvalid", || {
        std::hint::black_box(v.quantile_list(&qs, "linear").unwrap());
    });
    t("qlist99_nullable", || {
        std::hint::black_box(vn.quantile_list(&qs, "linear").unwrap());
    });
    let q5: Vec<f64> = vec![0.1, 0.25, 0.5, 0.75, 0.9];
    t("qlist5_allvalid", || {
        std::hint::black_box(v.quantile_list(&q5, "linear").unwrap());
    });
    // dump a digest for parity check
    let r = v.quantile_list(&qs, "linear").unwrap();
    let vals = r.column().values();
    let mut s = 0u64;
    for x in vals.iter() {
        if let Scalar::Float64(f) = x {
            s = s.wrapping_add(f.to_bits());
        }
    }
    println!("digest={s}");
}
