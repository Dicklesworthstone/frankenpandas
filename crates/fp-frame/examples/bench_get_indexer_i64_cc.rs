use std::time::Instant;

use fp_index::{Index, IndexLabel};
fn main() {
    let n: usize = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(1_000_000);
    let iters: usize = std::env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(50);
    let mut keys: Vec<i64> = (0..n as i64).collect();
    let mut st: u64 = 0x9E3779B97F4A7C15;
    for i in (1..n).rev() {
        st = st
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let j = (st >> 33) as usize % (i + 1);
        keys.swap(i, j);
    }
    let idx = Index::new(keys.iter().map(|&v| IndexLabel::Int64(v)).collect());
    let target = Index::new(
        (0..1000)
            .map(|j| IndexLabel::Int64((j * (n / 1000)) as i64))
            .collect(),
    );
    let mut best = u128::MAX;
    for _ in 0..iters {
        let t = Instant::now();
        let o = idx.get_indexer(&target);
        let e = t.elapsed().as_nanos();
        std::hint::black_box(&o);
        if e < best {
            best = e;
        }
    }
    println!(
        "get_indexer_i64 n={n} m=1000: best={best}ns ({:.4}ms)",
        best as f64 / 1e6
    );
}
