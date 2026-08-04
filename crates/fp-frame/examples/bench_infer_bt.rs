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
fn dh(s: &Series, d: &mut u64) {
    for v in s.column().values().iter() {
        let (tag, pay) = match v {
            Scalar::Float64(x) => (1u64, x.to_bits()),
            Scalar::Null(NullKind::NaN) => (2, 0),
            Scalar::Null(NullKind::NaT) => (3, 0),
            Scalar::Null(NullKind::Null) => (4, 0),
            Scalar::Int64(x) => (5, *x as u64),
            _ => (9, 0),
        };
        *d = d.wrapping_mul(1099511628211).wrapping_add(tag);
        *d = d.wrapping_mul(1099511628211).wrapping_add(pay);
    }
    for l in s.index().labels().iter() {
        if let fp_index::IndexLabel::Int64(x) = l {
            *d = d.wrapping_mul(1099511628211).wrapping_add(*x as u64);
        }
    }
}
fn main() {
    let n = 2_000_000usize;
    let idx = Index::from_range(0, n as i64, 1);
    // canonical NaN-missing (leading Null so from_inferred path)
    let vn = Series::new(
        "v",
        idx.clone(),
        Column::from_values(
            (0..n)
                .map(|i| {
                    if sm(i, 1).is_multiple_of(5) {
                        Scalar::Null(NullKind::NaN)
                    } else {
                        Scalar::Float64((sm(i, 9) % 1000000) as f64)
                    }
                })
                .collect(),
        )
        .unwrap(),
    )
    .unwrap();
    let positions: Vec<i64> = (0..n).map(|i| (sm(i, 3) % n as u64) as i64).collect();
    t("take_nan", || {
        std::hint::black_box(vn.take(&positions).unwrap());
    });
    t("sort_values_nan", || {
        std::hint::black_box(vn.sort_values_na(true, "last").unwrap());
    });
    t("drop_duplicates_nan", || {
        std::hint::black_box(vn.drop_duplicates().unwrap());
    });
    // DIFFERENTIAL over edge cases: canonical, present-Float64(NaN), Null(NaT), Null(Null), mixed w/ int
    let mut d = 0u64;
    for seed in 0..80u64 {
        let m = (sm(seed as usize, 3) % 1500) as usize;
        let vv: Vec<Scalar> = (0..m)
            .map(|i| {
                match sm(i, seed * 7 + 1) % 9 {
                    0 | 1 => Scalar::Null(NullKind::NaN),
                    2 => Scalar::Float64(f64::NAN), // present NaN (must stay Float64(NaN))
                    3 => Scalar::Null(NullKind::NaT), // must stay NaT
                    4 => Scalar::Null(NullKind::Null), // must stay Null
                    5 => Scalar::Int64((sm(i, seed + 2) % 100) as i64), // coerces to f64
                    _ => Scalar::Float64((sm(i, seed * 13 + 9) % 1000) as f64),
                }
            })
            .collect();
        let c = Column::from_values(vv);
        let c = match c {
            Ok(c) => c,
            Err(_) => continue,
        };
        let s = Series::new("s", Index::from_range(0, c.len() as i64, 1), c).unwrap();
        dh(&s, &mut d);
        let p: Vec<i64> = (0..s.len() as i64).rev().collect();
        dh(&s.take(&p).unwrap(), &mut d);
        dh(&s.sort_values_na(true, "last").unwrap(), &mut d);
        dh(&s.dropna().unwrap(), &mut d);
        if let Ok(dd) = s.drop_duplicates() {
            dh(&dd, &mut d);
        }
    }
    println!("variant_digest={d}");
}
