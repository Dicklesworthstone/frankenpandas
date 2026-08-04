use fp_columnar::Column;
use fp_frame::{Series, concat_series_with_ignore_index};
use fp_index::Index;
use fp_types::Scalar;
fn t(l: &str, mut f: impl FnMut()) {
    let mut b = u128::MAX;
    for _ in 0..6 {
        let x = std::time::Instant::now();
        f();
        b = b.min(x.elapsed().as_nanos());
    }
    println!("{l}: {:.2}ms", b as f64 / 1e6);
}
fn main() {
    let n = 1_000_000usize;
    let f: Vec<Series> = (0..5)
        .map(|k| {
            let idx = Index::from_range((k * n) as i64, ((k + 1) * n) as i64, 1);
            Series::new(
                "v",
                idx,
                Column::from_values(
                    (0..n)
                        .map(|i| Scalar::Float64(((i + k * n) as f64) * 0.5 - 1250000.0))
                        .collect(),
                )
                .unwrap(),
            )
            .unwrap()
        })
        .collect();
    let fi: Vec<Series> = (0..5)
        .map(|k| {
            let idx = Index::from_range((k * n) as i64, ((k + 1) * n) as i64, 1);
            Series::new(
                "v",
                idx,
                Column::from_values((0..n).map(|i| Scalar::Int64((i + k * n) as i64)).collect())
                    .unwrap(),
            )
            .unwrap()
        })
        .collect();
    let rf: Vec<&Series> = f.iter().collect();
    let ri: Vec<&Series> = fi.iter().collect();
    let cf = concat_series_with_ignore_index(&rf, true).unwrap();
    let ci = concat_series_with_ignore_index(&ri, true).unwrap();
    if let Scalar::Float64(v) = cf.var().unwrap() {
        println!("f64_var_bits={}", v.to_bits());
    }
    if let Scalar::Float64(v) = ci.var().unwrap() {
        println!("i64_var_bits={}", v.to_bits());
    }
    t("concat_f64+std", || {
        let c = concat_series_with_ignore_index(&rf, true).unwrap();
        std::hint::black_box(c.std().unwrap());
    });
    t("concat_i64+var", || {
        let c = concat_series_with_ignore_index(&ri, true).unwrap();
        std::hint::black_box(c.var().unwrap());
    });
}
