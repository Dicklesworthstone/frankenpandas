//! Bench for `Column::mode` on an all-valid Float64 column after fixing
//! mode_f64_wide (fixed cap≈1.5n + low-bit `& mask` hash → GROWABLE table sized
//! to the actual distinct count + FIBONACCI high-bit hash).
//!
//! Control here is the generic FxHashMap tally — the fair floor at every card
//! (the OLD open-addressing is pathological: ~2900ms at 2003 distinct, and WORSE
//! as cardinality rises because per-exponent-group clustering lengthens probe
//! chains, so it is not used as a control — it would hang the bench). Winner
//! checksums asserted equal. New should BEAT FxHashMap at low & high card and be
//! within a small factor in the mid-card band.
//!
//! Run: cargo run -p fp-columnar --release --example bench_mode_f64_fix -- 5000000 8

use std::io::Write;

use fp_columnar::Column;
use fp_types::Scalar;
use rustc_hash::FxHashMap;

fn fold(vals: &[Scalar]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for v in vals {
        let bits = match v {
            Scalar::Float64(x) => (if *x == 0.0 { 0.0 } else { *x }).to_bits(),
            _ => 0xDEAD_BEEF,
        };
        h ^= bits;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

fn control_fxhash(data: &[f64]) -> Vec<Scalar> {
    let mut counts: FxHashMap<u64, (u64, f64)> = FxHashMap::default();
    for &f in data {
        let kb = (if f == 0.0 { 0.0 } else { f }).to_bits();
        counts.entry(kb).and_modify(|e| e.0 += 1).or_insert((1, f));
    }
    let m = counts.values().map(|(c, _)| *c).max().unwrap_or(0);
    let mut w: Vec<f64> = counts
        .values()
        .filter_map(|&(c, f)| (c == m).then_some(f))
        .collect();
    w.sort_by(|a, b| a.total_cmp(b));
    w.into_iter().map(Scalar::Float64).collect()
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(5_000_000);
    let iters: usize = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(8);

    for &card in &[2_003usize, 100_000, 2_000_000, n] {
        let data: Vec<f64> = (0..n)
            .map(|i| {
                let h = (i as u64).wrapping_mul(2_654_435_761).wrapping_add(999);
                (h % card as u64) as f64
            })
            .collect();
        let col = Column::from_f64_values(data.clone());

        let mut best_t = u128::MAX;
        let mut ck_t: u64 = 0;
        for _ in 0..iters {
            let t = std::time::Instant::now();
            let out = col.mode().expect("mode");
            best_t = best_t.min(t.elapsed().as_nanos());
            ck_t = ck_t.wrapping_add(fold(out.values()));
        }
        let mut best_c = u128::MAX;
        let mut ck_c: u64 = 0;
        for _ in 0..iters {
            let t = std::time::Instant::now();
            let out = control_fxhash(&data);
            best_c = best_c.min(t.elapsed().as_nanos());
            ck_c = ck_c.wrapping_add(fold(&out));
        }
        assert_eq!(ck_t, ck_c, "card={card}: NEW != FxHashMap winners");
        println!(
            "mode_f64 card≈{card:>8} n={n} NEW={:>8.2}ms FxHashMap={:>8.2}ms new/fx={:.3}x",
            best_t as f64 / 1e6,
            best_c as f64 / 1e6,
            best_c as f64 / best_t as f64,
        );
        std::io::stdout().flush().ok();
    }
}
