//! Diagnostic: does count_distinct_i64_radix (the HIGH-card nunique path) cluster
//! its per-partition open-addressing tables for high-bit-only-varying keys?
//! Its per-partition index is `hashv(v) & mask` (LOW bits); for k<<34 keys
//! hashv = k·(golden<<34) has zero low bits ⇒ every value in a partition may map
//! to bucket 0. Compare nunique() (routes to radix at high card) vs FxHashSet for
//! SHIFTED (k<<34) vs NORMAL (k) high-cardinality keys.
//!
//! Run: cargo run -p fp-columnar --release --example bench_ndistinct_radix_diag -- 5000000 5

use std::io::Write;

use fp_columnar::Column;
use fp_types::Scalar;
use rustc_hash::FxHashSet;

fn best(iters: usize, mut f: impl FnMut() -> i64) -> f64 {
    let mut b = u128::MAX;
    for _ in 0..iters {
        let t = std::time::Instant::now();
        let r = f();
        b = b.min(t.elapsed().as_nanos());
        std::hint::black_box(r);
    }
    b as f64 / 1e6
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(5_000_000);
    let iters: usize = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(5);
    let card: u64 = 2_000_000; // high card ⇒ dispatcher uses radix

    for (label, shift) in [("shifted(k<<34)", 34u32), ("normal(k)", 0u32)] {
        let data: Vec<i64> = (0..n)
            .map(|i| {
                let h = (i as u64).wrapping_mul(2_654_435_761).wrapping_add(999);
                ((h % card) as i64) << shift
            })
            .collect();
        let col = Column::from_i64_values(data.clone());
        // Confirm wide (non-dense) path.
        assert!(fp_columnar::Column::from_i64_values(data.clone())
            .as_i64_slice()
            .is_some());

        let t_new = best(iters, || match col.nunique() {
            Scalar::Int64(x) => x,
            _ => 0,
        });
        let t_fx = best(iters, || {
            let mut s: FxHashSet<i64> = FxHashSet::default();
            for &v in &data {
                s.insert(v);
            }
            s.len() as i64
        });
        println!(
            "ndistinct {label:>16} n={n} card={card} nunique={:>7.2}ms FxHashSet={:>7.2}ms new/fx={:.3}x",
            t_new, t_fx, t_fx / t_new
        );
        std::io::stdout().flush().ok();
    }
}
