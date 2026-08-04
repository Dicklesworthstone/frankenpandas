//! Bench for `Column::duplicated` and `Column::nunique` on all-valid WIDE Int64
//! columns after fixing duplicated_first_i64_wide + count_distinct_i64_singletable
//! (fixed cap + low-bit `& mask` → growable table + Fibonacci high-bit hash).
//! Keys are `k << 34` ⇒ WIDE + HIGH-bit-only variation (the clustering case).
//!
//! duplicated: NEW=col.duplicated() vs FxHashSet flags (both build a Bool Column,
//! apples-to-apples). nunique: NEW=col.nunique() vs FxHashSet count (scalar, fair);
//! nunique's dispatcher routes only LOW/MID card to the fixed singletable (high
//! card ⇒ radix), so nunique is benched at 2003/100k. OLD timed at low card.
//!
//! Run: cargo run -p fp-columnar --release --example bench_dup_ndistinct_fix -- 5000000 6

use std::io::Write;

use fp_columnar::Column;
use fp_types::Scalar;
use rustc_hash::FxHashSet;

const GOLDEN: u64 = 0x9E37_79B9_7F4A_7C15;

fn dup_new_len(col: &Column) -> usize {
    let d = col.duplicated().expect("duplicated");
    std::hint::black_box(d.values().first());
    d.values().len()
}

fn dup_fxset(data: &[i64]) -> usize {
    let mut seen: FxHashSet<i64> = FxHashSet::default();
    let flags: Vec<bool> = data.iter().map(|&v| !seen.insert(v)).collect();
    let col = Column::from_bool_values(flags);
    std::hint::black_box(col.values().first());
    col.values().len()
}

fn dup_old(data: &[i64]) -> usize {
    const EMPTY: i64 = i64::MIN;
    let n = data.len();
    let mut flags = vec![false; n];
    let cap = n.saturating_add(n / 2).checked_next_power_of_two().unwrap_or(0);
    let mask = cap - 1;
    let mut keys = vec![EMPTY; cap];
    let mut ss = false;
    for (idx, &v) in data.iter().enumerate() {
        if v == EMPTY {
            flags[idx] = ss;
            ss = true;
            continue;
        }
        let mut p = ((v as u64).wrapping_mul(GOLDEN) as usize) & mask;
        loop {
            if keys[p] == EMPTY {
                keys[p] = v;
                break;
            }
            if keys[p] == v {
                flags[idx] = true;
                break;
            }
            p = (p + 1) & mask;
        }
    }
    let col = Column::from_bool_values(flags);
    std::hint::black_box(col.values().first());
    col.values().len()
}

fn ndistinct_fxset(data: &[i64]) -> usize {
    let mut seen: FxHashSet<i64> = FxHashSet::default();
    for &v in data {
        seen.insert(v);
    }
    seen.len()
}

fn best(iters: usize, mut f: impl FnMut() -> usize) -> f64 {
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
    let iters: usize = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(6);

    for (idx, &card) in [2_003usize, 100_000, 2_000_000].iter().enumerate() {
        let data: Vec<i64> = (0..n)
            .map(|i| {
                let h = (i as u64).wrapping_mul(2_654_435_761).wrapping_add(999);
                ((h % card as u64) as i64) << 34
            })
            .collect();
        let col = Column::from_i64_values(data.clone());

        let t_dup = best(iters, || dup_new_len(&col));
        let f_dup = best(iters, || dup_fxset(&data));
        let dup_old_str = if idx == 0 {
            let o = best(1, || dup_old(&data));
            format!(" OLD={o:.1}ms new/old={:.1}x", o / t_dup)
        } else {
            String::new()
        };
        println!(
            "duplicated card≈{card:>8} NEW={:>7.2}ms FxHashSet={:>7.2}ms new/fx={:.3}x{dup_old_str}",
            t_dup, f_dup, f_dup / t_dup
        );
        std::io::stdout().flush().ok();

        // nunique only where the dispatcher uses the (fixed) singletable path.
        if card <= 100_000 {
            let t_nu = best(iters, || match col.nunique() {
                Scalar::Int64(x) => x as usize,
                _ => 0,
            });
            let f_nu = best(iters, || ndistinct_fxset(&data));
            println!(
                "nunique    card≈{card:>8} NEW={:>7.2}ms FxHashSet={:>7.2}ms new/fx={:.3}x",
                t_nu, f_nu, f_nu / t_nu
            );
            std::io::stdout().flush().ok();
        }
    }
}
