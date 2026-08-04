//! Bench + golden for `Series.value_counts` on a high-cardinality Utf8 column —
//! the general counting path that now uses `FxHashMap` instead of std SipHash
//! (br-frankenpandas-g1de8). `n` rows drawn from `cardinality` distinct strings.
//! Golden = FNV-1a64 over the output (value, count) pairs, which pins both the
//! counts and their first-seen/sorted order so the hasher swap is proven
//! bit-transparent.
//!
//! Run: cargo run -p fp-frame --example bench_value_counts_utf8 --release -- 500000 5000 50

use std::time::Instant;

use fp_frame::Series;
use fp_index::IndexLabel;
use fp_types::Scalar;

fn build(n: usize, cardinality: usize) -> Series {
    let card = cardinality.max(1) as u64;
    let labels: Vec<IndexLabel> = (0..n as i64).map(IndexLabel::Int64).collect();
    let values: Vec<Scalar> = (0..n)
        .map(|row| {
            let mixed = (row as u64)
                .wrapping_mul(0x9E37_79B9_7F4A_7C15)
                .rotate_left(17)
                ^ (row as u64).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            Scalar::Utf8(format!("val_{:06x}", mixed % card))
        })
        .collect();
    Series::from_values("v", labels, values).expect("build series")
}

fn fnv1a64_update(h: &mut u64, bytes: &[u8]) {
    for b in bytes {
        *h ^= u64::from(*b);
        *h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
}

/// Hash the (value-label, count) pairs in output order so both content and
/// ordering are pinned.
fn digest(vc: &Series) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for (lbl, cnt) in vc.index().labels().iter().zip(vc.values().iter()) {
        match lbl {
            IndexLabel::Utf8(s) => fnv1a64_update(&mut h, s.as_bytes()),
            other => fnv1a64_update(&mut h, format!("{other:?}").as_bytes()),
        }
        let c = match cnt {
            Scalar::Int64(x) => *x,
            Scalar::Float64(x) => *x as i64,
            _ => i64::MIN,
        };
        fnv1a64_update(&mut h, &c.to_le_bytes());
    }
    h
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let n: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(500_000);
    let card: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(5_000);
    let iters: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(50);

    let series = build(n, card);
    let golden = digest(&series.value_counts().expect("value_counts"));

    let mut best = u128::MAX;
    for _ in 0..iters {
        let t = Instant::now();
        let vc = series.value_counts().expect("value_counts");
        let elapsed = t.elapsed().as_nanos();
        std::hint::black_box(&vc);
        if elapsed < best {
            best = elapsed;
        }
    }

    println!(
        "value_counts_utf8 n={n} cardinality={card} iters={iters}: best={best}ns golden={golden:016x}"
    );
}
