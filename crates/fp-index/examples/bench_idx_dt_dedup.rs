//! Index dedup-family ops over a Datetime64 index WITH duplicates @200k.
//! Run: bench_idx_dt_dedup <n> <op>
use fp_index::{DuplicateKeep, Index};

fn sm(i: usize, s: u64) -> u64 {
    let mut h = (i as u64).wrapping_add(s).wrapping_mul(0x9E3779B97F4A7C15);
    h = (h ^ (h >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    h ^= h >> 31;
    h
}
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(200_000);
    let op = a.get(2).map(String::as_str).unwrap_or("value_counts");
    let base = 1_577_836_800_000_000_000i64;
    let step = 60_000_000_000i64;
    // ~n/4 distinct timestamps, each repeated ~4x, shuffled order.
    let card = (n / 4).max(1) as u64;
    let ns: Vec<i64> = (0..n)
        .map(|i| base + (sm(i, 0) % card) as i64 * step)
        .collect();
    let idx = Index::from_datetime64(ns);
    let mut best = u128::MAX;
    for _ in 0..6 {
        let t = std::time::Instant::now();
        match op {
            "value_counts" => {
                std::hint::black_box(idx.value_counts());
            }
            "duplicated" => {
                std::hint::black_box(idx.duplicated(DuplicateKeep::First));
            }
            "drop_duplicates" => {
                std::hint::black_box(idx.drop_duplicates());
            }
            "nunique" => {
                std::hint::black_box(idx.nunique());
            }
            "unique" => {
                std::hint::black_box(idx.unique());
            }
            "has_duplicates" => {
                std::hint::black_box(idx.has_duplicates());
            }
            _ => panic!("op"),
        }
        let e = t.elapsed().as_nanos();
        if e < best {
            best = e;
        }
    }
    println!("idx_dt_{op} n={n}: best={best}ns");
}
