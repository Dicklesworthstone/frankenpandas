//! Bench + golden for `Series.drop_duplicates()` on an Int64 column with many
//! duplicate values — a flagship dedup workload. Exercises the FxHashSet first-seen
//! dedup path (br-frankenpandas-6vep3 vein). `n` rows over `cardinality` distinct
//! values. Golden = FNV-1a64 over the kept (index-label, value) pairs (first-seen
//! order), pinning that the hasher swap stayed bit-transparent.
//!
//! Run: cargo run -p fp-frame --example bench_drop_duplicates --release -- 1000000 1000 30

use std::time::Instant;

use fp_frame::Series;
use fp_index::IndexLabel;
use fp_types::Scalar;

fn build(n: usize, cardinality: usize) -> Series {
    let card = cardinality.max(1) as i64;
    let labels: Vec<IndexLabel> = (0..n as i64).map(IndexLabel::Int64).collect();
    let values: Vec<Scalar> = (0..n as i64).map(|v| Scalar::Int64(v % card)).collect();
    Series::from_values("v", labels, values).expect("build series")
}

fn fnv1a64_update(h: &mut u64, bytes: &[u8]) {
    for b in bytes {
        *h ^= u64::from(*b);
        *h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
}

fn digest(s: &Series) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for (lbl, val) in s.index().labels().iter().zip(s.values().iter()) {
        let l = match lbl {
            IndexLabel::Int64(x) => *x,
            _ => i64::MIN,
        };
        let v = match val {
            Scalar::Int64(x) => *x,
            _ => i64::MIN,
        };
        fnv1a64_update(&mut h, &l.to_le_bytes());
        fnv1a64_update(&mut h, &v.to_le_bytes());
    }
    h
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let n: usize = args
        .get(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(1_000_000);
    let card: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(1_000);
    let iters: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(30);

    let series = build(n, card);
    let golden = digest(&series.drop_duplicates().expect("drop_duplicates"));

    let mut best = u128::MAX;
    for _ in 0..iters {
        let t = Instant::now();
        let out = series.drop_duplicates().expect("drop_duplicates");
        let e = t.elapsed().as_nanos();
        std::hint::black_box(&out);
        if e < best {
            best = e;
        }
    }

    println!(
        "drop_duplicates n={n} cardinality={card} iters={iters}: best={best}ns golden={golden:016x}"
    );
}
