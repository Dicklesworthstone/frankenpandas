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
    let parts: Vec<Series> = (0..5)
        .map(|k| {
            let idx = Index::from_range((k * n) as i64, ((k + 1) * n) as i64, 1);
            Series::new(
                "v",
                idx,
                Column::from_values(
                    (0..n)
                        .map(|i| Scalar::Float64(((i + k * n) as f64) * 0.5 - 3.0))
                        .collect(),
                )
                .unwrap(),
            )
            .unwrap()
        })
        .collect();
    let refs: Vec<&Series> = parts.iter().collect();
    let c = concat_series_with_ignore_index(&refs, true).unwrap();
    if let Scalar::Float64(s) = c.sum().unwrap() {
        println!("sum_bits={}", s.to_bits());
    }
    if let Scalar::Float64(m) = c.mean().unwrap() {
        println!("mean_bits={}", m.to_bits());
    }
    t("concat+sum", || {
        let c = concat_series_with_ignore_index(&refs, true).unwrap();
        std::hint::black_box(c.sum().unwrap());
    });
    t("concat+mean", || {
        let c = concat_series_with_ignore_index(&refs, true).unwrap();
        std::hint::black_box(c.mean().unwrap());
    });
}
