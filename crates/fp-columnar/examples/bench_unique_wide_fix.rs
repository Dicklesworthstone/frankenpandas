//! Bench for `Column::unique` on all-valid WIDE Float64 / Int64 columns after
//! fixing unique_{f64,i64}_wide (fixed cap≈1.5n + low-bit `& mask` → growable
//! table + Fibonacci high-bit hash). f64 uses float-of-integer values (variation
//! in HIGH to_bits() bits — the clustering case); i64 spreads ×1e9 (wide range).
//!
//! NEW = col.unique(); FxHashSet = fair floor; OLD = pre-fix fixed-cap + `& mask`
//! (timed only at low card — pathological). Output order/dedup asserted equal.
//!
//! Run: cargo run -p fp-columnar --release --example bench_unique_wide_fix -- 5000000 6

use std::io::Write;

use fp_columnar::Column;
use fp_types::Scalar;
use rustc_hash::FxHashSet;

fn f64_new(col: &Column) -> Vec<u64> {
    col.unique()
        .expect("unique")
        .values()
        .iter()
        .map(|v| match v {
            Scalar::Float64(x) => x.to_bits(),
            _ => 0xDEAD,
        })
        .collect()
}

fn f64_fxset(data: &[f64]) -> Vec<u64> {
    let mut seen: FxHashSet<u64> = FxHashSet::default();
    let mut out = Vec::new();
    for &f in data {
        let kb = (if f == 0.0 { 0.0 } else { f }).to_bits();
        if seen.insert(kb) {
            out.push(f.to_bits());
        }
    }
    out
}

fn f64_old(data: &[f64]) -> usize {
    const EMPTY: i64 = i64::MIN;
    let cap = data.len().saturating_add(data.len() / 2).checked_next_power_of_two().unwrap_or(0);
    let mask = cap - 1;
    let mut keys = vec![EMPTY; cap];
    let mut cnt = 0usize;
    for &f in data {
        let kb = (if f == 0.0 { 0.0 } else { f }).to_bits() as i64;
        let mut p = ((kb as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) as usize) & mask;
        loop {
            if keys[p] == EMPTY {
                keys[p] = kb;
                cnt += 1;
                break;
            }
            if keys[p] == kb {
                break;
            }
            p = (p + 1) & mask;
        }
    }
    cnt
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(5_000_000);
    let iters: usize = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(6);

    for (idx, &card) in [2_003usize, 100_000, 2_000_000].iter().enumerate() {
        // Float-of-integer ⇒ variation in the HIGH to_bits() bits (the case the
        // old low-bit `& mask` hash collapsed into per-exponent buckets).
        let data: Vec<f64> = (0..n)
            .map(|i| {
                let h = (i as u64).wrapping_mul(2_654_435_761).wrapping_add(999);
                (h % card as u64) as f64
            })
            .collect();
        let col = Column::from_f64_values(data.clone());

        let mut best_t = u128::MAX;
        for _ in 0..iters {
            let t = std::time::Instant::now();
            let w = f64_new(&col);
            best_t = best_t.min(t.elapsed().as_nanos());
            std::hint::black_box(&w);
        }
        let mut best_f = u128::MAX;
        for _ in 0..iters {
            let t = std::time::Instant::now();
            let w = f64_fxset(&data);
            best_f = best_f.min(t.elapsed().as_nanos());
            std::hint::black_box(&w);
        }
        assert_eq!(f64_new(&col), f64_fxset(&data), "card={card}: NEW != FxHashSet");
        let old_str = if idx == 0 {
            let t = std::time::Instant::now();
            let c = f64_old(&data);
            let old_ms = t.elapsed().as_nanos() as f64 / 1e6;
            std::hint::black_box(c);
            format!(" OLD={old_ms:.1}ms new/old={:.1}x", old_ms / (best_t as f64 / 1e6))
        } else {
            String::new()
        };
        println!(
            "unique_f64 card≈{card:>8} n={n} NEW={:>7.2}ms FxHashSet={:>7.2}ms new/fx={:.3}x{old_str}",
            best_t as f64 / 1e6,
            best_f as f64 / 1e6,
            best_f as f64 / best_t as f64,
        );
        std::io::stdout().flush().ok();
    }
}
