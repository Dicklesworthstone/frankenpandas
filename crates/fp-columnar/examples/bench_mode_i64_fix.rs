//! Bench for `Column::mode` on an all-valid WIDE-range Int64 column after fixing
//! mode_i64_wide (fixed cap≈1.5n + low-bit `& mask` → growable table + Fibonacci
//! high-bit hash). Values are spread ×1e9 so they miss the dense direct-address
//! path and exercise mode_i64_wide (also the Datetime64/Timedelta64 mode path).
//!
//! NEW = col.mode(); FxHashMap = fair floor at every card; OLD = the pre-fix
//! fixed-cap + `& mask` open-addressing (timed only at low card for a headline —
//! it is pathological). Winner checksums asserted equal.
//!
//! Run: cargo run -p fp-columnar --release --example bench_mode_i64_fix -- 5000000 8

use std::io::Write;

use fp_columnar::Column;
use fp_types::Scalar;
use rustc_hash::FxHashMap;

fn fold(vals: &[Scalar]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for v in vals {
        let bits = match v {
            Scalar::Int64(x) => *x as u64,
            _ => 0xDEAD_BEEF,
        };
        h ^= bits;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

fn control_fxhash(data: &[i64]) -> Vec<i64> {
    let mut counts: FxHashMap<i64, u64> = FxHashMap::default();
    for &v in data {
        *counts.entry(v).or_insert(0) += 1;
    }
    let m = counts.values().copied().max().unwrap_or(0);
    let mut w: Vec<i64> = counts.iter().filter_map(|(&k, &c)| (c == m).then_some(k)).collect();
    w.sort_unstable();
    w
}

fn old_openaddr(data: &[i64]) -> Vec<i64> {
    const EMPTY: i64 = i64::MIN;
    let cap = data.len().saturating_add(data.len() / 2).checked_next_power_of_two().unwrap_or(0);
    let mask = cap - 1;
    let mut keys = vec![EMPTY; cap];
    let mut cnt = vec![0u32; cap];
    let mut sc: u64 = 0;
    for &v in data {
        if v == EMPTY {
            sc += 1;
            continue;
        }
        let mut p = ((v as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) as usize) & mask;
        loop {
            if keys[p] == EMPTY {
                keys[p] = v;
                cnt[p] = 1;
                break;
            }
            if keys[p] == v {
                cnt[p] += 1;
                break;
            }
            p = (p + 1) & mask;
        }
    }
    let mut mc = sc;
    for p in 0..cap {
        if keys[p] != EMPTY {
            mc = mc.max(u64::from(cnt[p]));
        }
    }
    let mut w: Vec<i64> = Vec::new();
    if sc > 0 && sc == mc {
        w.push(EMPTY);
    }
    for p in 0..cap {
        if keys[p] != EMPTY && u64::from(cnt[p]) == mc {
            w.push(keys[p]);
        }
    }
    w.sort_unstable();
    w
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(5_000_000);
    let iters: usize = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(8);

    for (idx, &card) in [2_003usize, 100_000, 2_000_000, n].iter().enumerate() {
        // Spread ×1e9 ⇒ wide range (misses the dense path) and high-bit variation.
        let data: Vec<i64> = (0..n)
            .map(|i| {
                let h = (i as u64).wrapping_mul(2_654_435_761).wrapping_add(999);
                (h % card as u64) as i64 * 1_000_000_000
            })
            .collect();
        let col = Column::from_i64_values(data.clone());

        let mut best_t = u128::MAX;
        let mut ck_t: u64 = 0;
        for _ in 0..iters {
            let t = std::time::Instant::now();
            let out = col.mode().expect("mode");
            best_t = best_t.min(t.elapsed().as_nanos());
            ck_t = ck_t.wrapping_add(fold(out.values()));
        }
        let mut best_f = u128::MAX;
        for _ in 0..iters {
            let t = std::time::Instant::now();
            let w = control_fxhash(&data);
            best_f = best_f.min(t.elapsed().as_nanos());
            std::hint::black_box(&w);
        }
        // OLD only at the lowest card (headline); it is pathological.
        let old_str = if idx == 0 {
            let t = std::time::Instant::now();
            let w = old_openaddr(&data);
            let old_ms = t.elapsed().as_nanos() as f64 / 1e6;
            std::hint::black_box(&w);
            format!(" OLD={old_ms:.2}ms new/old={:.1}x", old_ms / (best_t as f64 / 1e6))
        } else {
            String::new()
        };
        let _ = ck_t;
        // One-time correctness: NEW winners == FxHashMap reference winners.
        let new_w: Vec<i64> = col
            .mode()
            .expect("mode")
            .values()
            .iter()
            .map(|v| match v {
                Scalar::Int64(x) => *x,
                o => panic!("not int: {o:?}"),
            })
            .collect();
        assert_eq!(new_w, control_fxhash(&data), "card={card}: NEW != FxHashMap winners");
        println!(
            "mode_i64 card≈{card:>8} n={n} NEW={:>7.2}ms FxHashMap={:>7.2}ms new/fx={:.3}x{old_str}",
            best_t as f64 / 1e6,
            best_f as f64 / 1e6,
            best_f as f64 / best_t as f64,
        );
        std::io::stdout().flush().ok();
    }
}
