use fp_columnar::Column;
use fp_frame::{Series, concat_series_with_ignore_index};
use fp_index::Index;
use fp_types::Scalar;
fn t(l: &str, mut f: impl FnMut()) {
    let mut b = u128::MAX;
    for _ in 0..5 {
        let x = std::time::Instant::now();
        f();
        b = b.min(x.elapsed().as_nanos());
    }
    println!("{l}: {:.2}ms", b as f64 / 1e6);
}
fn dig(s: &Series) -> u64 {
    let mut h = 1469598103934665603u64;
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
    let n = 1_000_000usize;
    let idx = Index::from_range(0, n as i64, 1);
    let sf = Series::new(
        "v",
        idx.clone(),
        Column::from_values((0..n).map(|i| Scalar::Float64(i as f64)).collect()).unwrap(),
    )
    .unwrap();
    let si1 = Series::new(
        "v",
        idx.clone(),
        Column::from_values((0..n).map(|i| Scalar::Int64(i as i64)).collect()).unwrap(),
    )
    .unwrap();
    let si2 = Series::new(
        "v",
        idx.clone(),
        Column::from_values((0..n).map(|i| Scalar::Int64(i as i64 + 5)).collect()).unwrap(),
    )
    .unwrap();
    let mixed = vec![&sf, &si1, &si2];
    let c = concat_series_with_ignore_index(&mixed, true).unwrap();
    println!(
        "mixed_dtype={:?} mixed_digest={}",
        c.column().dtype(),
        dig(&c)
    );
    t("concat_mixed", || {
        std::hint::black_box(concat_series_with_ignore_index(&mixed, true).unwrap());
    });
}
