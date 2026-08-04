//! Probe: Series.value_counts() on all-unique float64 (harness workload:
//! rng.random(n)*1e6). Replicates benches/vs_pandas_harness.py value_counts.
use std::{hint::black_box, time::Instant};

use fp_columnar::Column;
use fp_frame::Series;
use fp_index::{Index, IndexLabel};
fn bench(name: &str, it: usize, mut f: impl FnMut() -> usize) {
    for _ in 0..3 {
        black_box(f());
    }
    let st = Instant::now();
    let mut k = 0usize;
    for _ in 0..it {
        k ^= black_box(f());
    }
    println!(
        "{name:30}: {:.3} ms/call (k={k})",
        st.elapsed().as_secs_f64() * 1000.0 / it as f64
    );
}
fn mkdata(n: usize) -> Vec<f64> {
    // mimic numpy default_rng(seed).random(n)*1e6 — continuous, all-unique.
    let mut z = 0x2545F4914F6CDD1Du64;
    (0..n)
        .map(|_| {
            z ^= z << 13;
            z ^= z >> 7;
            z ^= z << 17;
            (z >> 11) as f64 / (1u64 << 53) as f64 * 1_000_000.0
        })
        .collect()
}
fn main() {
    for n in [100_000usize, 1_000_000] {
        let v = mkdata(n);
        let idx = Index::new((0..n as i64).map(IndexLabel::Int64).collect());
        let s = Series::new("col_0".to_string(), idx, Column::from_f64_values(v.clone())).unwrap();
        bench(
            &format!("value_counts_{}", n),
            if n >= 1_000_000 { 15 } else { 60 },
            || s.value_counts().map(|r| r.len()).unwrap_or(0),
        );
    }
}
