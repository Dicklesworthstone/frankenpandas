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
    // tie-heavy: value in [0,1000), max=999 repeated MANY times across lanes/chunks;
    // sawtooth so min=0 repeated too. Stresses first-occurrence tie-break.
    let f: Vec<Series> = (0..5)
        .map(|k| {
            let idx = Index::from_range((k * n) as i64, ((k + 1) * n) as i64, 1);
            Series::new(
                "v",
                idx,
                Column::from_values(
                    (0..n)
                        .map(|i| Scalar::Float64(((i + k * n) % 1000) as f64))
                        .collect(),
                )
                .unwrap(),
            )
            .unwrap()
        })
        .collect();
    let rf: Vec<&Series> = f.iter().collect();
    let c = concat_series_with_ignore_index(&rf, true).unwrap();
    println!("idxmax_label={:?}", c.idxmax().unwrap());
    println!("idxmin_label={:?}", c.idxmin().unwrap());
    t("concat+idxmax", || {
        let c = concat_series_with_ignore_index(&rf, true).unwrap();
        std::hint::black_box(c.idxmax().unwrap());
    });
    t("concat+idxmin", || {
        let c = concat_series_with_ignore_index(&rf, true).unwrap();
        std::hint::black_box(c.idxmin().unwrap());
    });
}
