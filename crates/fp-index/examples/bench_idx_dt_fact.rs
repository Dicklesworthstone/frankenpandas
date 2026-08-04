//! Index factorize / get_indexer_non_unique over a Datetime64 index @200k.
//! Run: bench_idx_dt_fact <n> <op>
use fp_index::Index;

fn sm(i: usize, s: u64) -> u64 {
    let mut h = (i as u64).wrapping_add(s).wrapping_mul(0x9E3779B97F4A7C15);
    h = (h ^ (h >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    h ^= h >> 31;
    h
}
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(200_000);
    let op = a.get(2).map(String::as_str).unwrap_or("factorize");
    let base = 1_577_836_800_000_000_000i64;
    let step = 60_000_000_000i64;
    let card = (n / 4).max(1) as u64;
    let ns: Vec<i64> = (0..n)
        .map(|i| base + (sm(i, 0) % card) as i64 * step)
        .collect();
    let idx = Index::from_datetime64(ns);
    // target for get_indexer_non_unique: distinct subset
    let tgt_ns: Vec<i64> = (0..card as usize).map(|i| base + i as i64 * step).collect();
    let target = Index::from_datetime64(tgt_ns);
    let mut best = u128::MAX;
    for _ in 0..6 {
        let t = std::time::Instant::now();
        match op {
            "factorize" => {
                std::hint::black_box(idx.factorize());
            }
            "gin" => {
                std::hint::black_box(idx.get_indexer_non_unique(&target));
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
