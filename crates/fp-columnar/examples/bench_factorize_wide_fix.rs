//! Bench for `Column::factorize` on an all-valid WIDE Int64 column after fixing
//! factorize_i64_wide (fixed cap≈1.5n + low-bit `& mask` → growable table +
//! Fibonacci high-bit hash). Values are `k << 34` ⇒ WIDE range AND variation only
//! in HIGH bits — the case the old low-bit hash collapsed into one bucket.
//!
//! NEW = col.factorize_with_options(false,true); FxHashMap = fair floor; OLD =
//! pre-fix fixed-cap + `& mask` (timed only at low card — pathological). codes
//! length asserted equal.
//!
//! Run: cargo run -p fp-columnar --release --example bench_factorize_wide_fix -- 5000000 6

use std::io::Write;

use fp_columnar::Column;
use fp_types::Scalar;
use rustc_hash::FxHashMap;

fn new_codes_len(col: &Column) -> (usize, usize) {
    let (codes, uniques) = col
        .factorize_with_options(false, true)
        .expect("factorize");
    let c = codes.values().len();
    let u = uniques.values().len();
    // touch a value so nothing is optimized away
    std::hint::black_box(codes.values().first());
    (c, u)
}

fn fxhash_factorize(data: &[i64]) -> usize {
    let mut map: FxHashMap<i64, i64> = FxHashMap::default();
    let mut codes: Vec<i64> = Vec::with_capacity(data.len());
    let mut uniques: Vec<i64> = Vec::new();
    for &v in data {
        match map.get(&v) {
            Some(&c) => codes.push(c),
            None => {
                let c = uniques.len() as i64;
                map.insert(v, c);
                uniques.push(v);
                codes.push(c);
            }
        }
    }
    // Apples-to-apples: NEW (factorize_with_options) also builds output Columns,
    // so wrap here too (same from_i64_values_owned the impl uses).
    let cc = Column::from_i64_values_owned(codes);
    let uc = Column::from_i64_values_owned(uniques);
    std::hint::black_box(cc.values().first());
    uc.values().len()
}

fn old_factorize(data: &[i64]) -> usize {
    const EMPTY: i64 = i64::MIN;
    let cap = data.len().saturating_add(data.len() / 2).checked_next_power_of_two().unwrap_or(0);
    let mask = cap - 1;
    let mut keys = vec![EMPTY; cap];
    let mut code_at = vec![0u32; cap];
    let mut codes: Vec<i64> = Vec::with_capacity(data.len());
    let mut nuniq = 0i64;
    let mut min_seen = false;
    for &v in data {
        if v == EMPTY {
            if !min_seen {
                min_seen = true;
                nuniq += 1;
            }
            codes.push(0);
            continue;
        }
        let mut idx = ((v as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) as usize) & mask;
        loop {
            if keys[idx] == EMPTY {
                keys[idx] = v;
                code_at[idx] = nuniq as u32;
                codes.push(nuniq);
                nuniq += 1;
                break;
            }
            if keys[idx] == v {
                codes.push(i64::from(code_at[idx]));
                break;
            }
            idx = (idx + 1) & mask;
        }
    }
    std::hint::black_box(&codes);
    nuniq as usize
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(5_000_000);
    let iters: usize = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(6);

    for (idx, &card) in [2_003usize, 100_000, 2_000_000].iter().enumerate() {
        // k << 34 ⇒ wide range + HIGH-bit-only variation.
        let data: Vec<i64> = (0..n)
            .map(|i| {
                let h = (i as u64).wrapping_mul(2_654_435_761).wrapping_add(999);
                ((h % card as u64) as i64) << 34
            })
            .collect();
        let col = Column::from_i64_values(data.clone());

        let mut best_t = u128::MAX;
        for _ in 0..iters {
            let t = std::time::Instant::now();
            let r = new_codes_len(&col);
            best_t = best_t.min(t.elapsed().as_nanos());
            std::hint::black_box(r);
        }
        let mut best_f = u128::MAX;
        for _ in 0..iters {
            let t = std::time::Instant::now();
            let r = fxhash_factorize(&data);
            best_f = best_f.min(t.elapsed().as_nanos());
            std::hint::black_box(r);
        }
        let old_str = if idx == 0 {
            let t = std::time::Instant::now();
            let r = old_factorize(&data);
            let old_ms = t.elapsed().as_nanos() as f64 / 1e6;
            std::hint::black_box(r);
            format!(" OLD={old_ms:.1}ms new/old={:.1}x", old_ms / (best_t as f64 / 1e6))
        } else {
            String::new()
        };
        println!(
            "factorize_i64 card≈{card:>8} n={n} NEW={:>7.2}ms FxHashMap={:>7.2}ms new/fx={:.3}x{old_str}",
            best_t as f64 / 1e6,
            best_f as f64 / 1e6,
            best_f as f64 / best_t as f64,
        );
        std::io::stdout().flush().ok();
    }
}
