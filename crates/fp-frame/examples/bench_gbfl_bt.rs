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
    for _ in 0..4 {
        let x = std::time::Instant::now();
        f();
        b = b.min(x.elapsed().as_nanos());
    }
    println!("{l}: {:.2}ms", b as f64 / 1e6);
}
fn main() {
    let n = 1_000_000usize;
    let idx = Index::from_range(0, n as i64, 1);
    let k = Series::new(
        "k",
        idx.clone(),
        Column::from_values(
            (0..n)
                .map(|i| Scalar::Utf8(format!("k{}", sm(i, 1) % 500)))
                .collect(),
        )
        .unwrap(),
    )
    .unwrap();
    let v = Series::new(
        "v",
        idx.clone(),
        Column::from_values(
            (0..n)
                .map(|i| Scalar::Float64((sm(i, 7) % 100000) as f64))
                .collect(),
        )
        .unwrap(),
    )
    .unwrap();
    t("sgb_agg_firstlast", || {
        std::hint::black_box(
            v.groupby(&k)
                .unwrap()
                .agg(&["first", "last", "sum"])
                .unwrap(),
        );
    });
    let r = v
        .groupby(&k)
        .unwrap()
        .agg(&["first", "last", "sum", "mean", "min", "max"])
        .unwrap();
    let mut d = 0u64;
    for c in ["first", "last", "sum", "mean", "min", "max"] {
        if let Some(col) = r.column(c) {
            for x in col.values().iter() {
                if let Scalar::Float64(f) = x {
                    d = d.wrapping_mul(1099511628211).wrapping_add(f.to_bits());
                }
            }
        }
    }
    println!("digest={d}");
}
