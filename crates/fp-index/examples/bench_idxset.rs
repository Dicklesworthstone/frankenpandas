use fp_index::{Index, IndexLabel};
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
    let n = 1_000_000usize;
    let a: Vec<IndexLabel> = (0..n)
        .map(|i| IndexLabel::Int64(((sm(i, 1) % (2 * n as u64)) as i64) * 2654435761))
        .collect();
    let b: Vec<IndexLabel> = (0..n)
        .map(|i| IndexLabel::Int64(((sm(i, 2) % (2 * n as u64)) as i64) * 2654435761))
        .collect();
    let ia = Index::new(a);
    let ib = Index::new(b);
    timeit("Index.intersection wide-i64", || {
        std::hint::black_box(ia.intersection(&ib));
    });
    timeit("Index.union wide-i64", || {
        std::hint::black_box(ia.union(&ib));
    });
    timeit("Index.difference wide-i64", || {
        std::hint::black_box(ia.difference(&ib));
    });
    timeit("Index.get_indexer wide-i64", || {
        std::hint::black_box(ia.get_indexer(&ib));
    });
}
