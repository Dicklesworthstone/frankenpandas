//! Index argsort / sort_values over an UNSORTED Datetime64 index @200k.
//! Run: bench_idx_dt_sort <n> <op>
use fp_index::Index;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(200_000);
    let op = a.get(2).map(String::as_str).unwrap_or("sort_values");
    let base = 1_577_836_800_000_000_000i64;
    let step = 60_000_000_000i64;
    let mut ns: Vec<i64> = (0..n).map(|i| base + i as i64 * step).collect();
    // splitmix Fisher-Yates shuffle -> Unsorted
    let st = |x: &mut u64| -> u64 {
        *x = x.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = *x;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    };
    let mut s = 1u64;
    for i in (1..n).rev() {
        let j = (st(&mut s) as usize) % (i + 1);
        ns.swap(i, j);
    }
    let idx = Index::from_datetime64(ns);
    let mut best = u128::MAX;
    for _ in 0..6 {
        let t = std::time::Instant::now();
        match op {
            "argsort" => {
                std::hint::black_box(idx.argsort());
            }
            "sort_values" => {
                std::hint::black_box(idx.sort_values());
            }
            _ => {
                eprintln!("unsupported op: {op}");
                std::process::exit(2);
            }
        }
        let e = t.elapsed().as_nanos();
        if e < best {
            best = e;
        }
    }
    println!("idx_dt_{op} n={n}: best={best}ns");
}
