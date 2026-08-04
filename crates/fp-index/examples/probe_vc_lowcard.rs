use std::{hint::black_box, time::Instant};

use fp_index::{Index, IndexLabel};
fn main() {
    let n = 1_000_000usize;
    let mut z = 0x1u64;
    let mut vals: Vec<i64> = (0..n).map(|i| (i as i64) % 100).collect();
    for i in (1..n).rev() {
        z ^= z << 13;
        z ^= z >> 7;
        z ^= z << 17;
        let j = (z as usize) % (i + 1);
        vals.swap(i, j);
    }
    let idx = Index::new(vals.iter().map(|&v| IndexLabel::Int64(v)).collect());
    for _ in 0..3 {
        black_box(idx.value_counts().len());
    }
    let it = 20;
    let s = Instant::now();
    let mut k = 0usize;
    for _ in 0..it {
        k ^= black_box(idx.value_counts().len());
    }
    println!(
        "value_counts lowcard(100): {:.3} ms (k={k})",
        s.elapsed().as_secs_f64() * 1000.0 / it as f64
    );
}
