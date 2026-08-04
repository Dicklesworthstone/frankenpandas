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
fn main() {
    let n = 2_000_000usize;
    let idx = Index::from_range(0, n as i64, 1);
    let col = Column::from_values((0..n).map(|i| Scalar::Float64(i as f64)).collect()).unwrap();
    let v = Series::new("v", idx.clone(), col.clone()).unwrap();
    let mut pos_u: Vec<usize> = (0..n).map(|i| (sm(i, 3) % n as u64) as usize).collect();
    let mut pos_i: Vec<i64> = pos_u.iter().map(|&p| p as i64).collect();
    // sorted variant
    pos_u.sort_unstable();
    pos_i.sort_unstable();
    t("col.take_positions (sorted)", || {
        std::hint::black_box(col.take_positions(&pos_u));
    });
    t("index.take (sorted)", || {
        std::hint::black_box(idx.take(&pos_u));
    });
    t("Series::take (sorted)", || {
        std::hint::black_box(v.take(&pos_i).unwrap());
    });
    println!(
        "out dtype after col.take: {:?}",
        col.take_positions(&pos_u).dtype()
    );
}
