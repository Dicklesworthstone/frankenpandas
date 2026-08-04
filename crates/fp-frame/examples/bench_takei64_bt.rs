use fp_columnar::Column;
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
    let col = Column::from_values((0..n).map(|i| Scalar::Int64(i as i64)).collect()).unwrap();
    let mut pos: Vec<usize> = (0..n).map(|i| (sm(i, 3) % n as u64) as usize).collect();
    pos.sort_unstable();
    t("col.take_positions i64 (sorted)", || {
        std::hint::black_box(col.take_positions(&pos));
    });
    println!("dtype={:?}", col.take_positions(&pos).dtype());
}
