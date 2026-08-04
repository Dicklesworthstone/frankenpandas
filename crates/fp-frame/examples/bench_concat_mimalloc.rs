//! Same concat workload as bench_concat, but with mimalloc as the global allocator.
//! Tests the gauntlet hypothesis that concat's 24× loss is the fresh-8MB-Vec
//! allocation/page-fault floor (glibc malloc mmaps large allocs; numpy pools them).
//! If mimalloc (a pooling allocator) is dramatically faster, the floor is allocator-bound
//! and a workspace global allocator is the fix for the rebuild-class losses.
//!
//! Run: cargo run -p fp-frame --example bench_concat_mimalloc --release -- 1000000 8 30

use std::time::Instant;

use fp_frame::{Series, concat_series_with_ignore_index};
use fp_index::IndexLabel;
use fp_types::Scalar;
use mimalloc::MiMalloc;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

fn build_one(m: usize, base: i64) -> Series {
    Series::from_values(
        "v",
        (0..m as i64).map(IndexLabel::Int64).collect(),
        (0..m as i64).map(|x| Scalar::Int64(x + base)).collect(),
    )
    .unwrap()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let total: usize = args
        .get(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(1_000_000);
    let k: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(8);
    let iters: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(30);
    let m = total / k.max(1);
    let parts: Vec<Series> = (0..k).map(|i| build_one(m, (i * m) as i64)).collect();
    let refs: Vec<&Series> = parts.iter().collect();

    let mut best = u128::MAX;
    for _ in 0..iters {
        let t = Instant::now();
        let out = concat_series_with_ignore_index(&refs, true).expect("concat");
        let e = t.elapsed().as_nanos();
        std::hint::black_box(&out);
        if e < best {
            best = e;
        }
    }
    println!("concat_mimalloc total={total} k={k} m={m} iters={iters}: best={best}ns");
}
