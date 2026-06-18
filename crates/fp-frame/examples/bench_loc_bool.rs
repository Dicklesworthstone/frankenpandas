//! Bench + golden for `Series.loc_bool(mask)` on a large Int64 column — the
//! flagship `df[mask]` / `s[s > x]` boolean-indexing workload. Exercises the
//! collect-positions + zero-copy `Index::take` + `Column::take_positions` lever
//! (br-frankenpandas-t0y8n): one position scan, then typed gathers, instead of a
//! per-kept-row IndexLabel + Scalar clone.
//! Golden = FNV-1a64 over the selected (index-label, value) pairs.
//!
//! Run: cargo run -p fp-frame --example bench_loc_bool --release -- 2000000 100

use std::time::Instant;

use fp_frame::Series;
use fp_index::IndexLabel;
use fp_types::Scalar;

fn build(n: usize) -> Series {
    let labels: Vec<IndexLabel> = (0..n as i64).map(IndexLabel::Int64).collect();
    let values: Vec<Scalar> = (0..n as i64).map(Scalar::Int64).collect();
    Series::from_values("v", labels, values).expect("build series")
}

/// Deterministic ~50% mask: keep rows whose mixed hash is even.
fn build_mask(n: usize) -> Vec<bool> {
    (0..n)
        .map(|row| {
            let mixed = (row as u64)
                .wrapping_mul(0x9E37_79B9_7F4A_7C15)
                .rotate_left(23)
                ^ (row as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
            mixed & 1 == 0
        })
        .collect()
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
    let n: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(2_000_000);
    let iters: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(100);

    let series = build(n);
    let mask = build_mask(n);
    let golden = digest(&series.loc_bool(&mask).expect("loc_bool"));

    let mut best = u128::MAX;
    for _ in 0..iters {
        let t = Instant::now();
        let out = series.loc_bool(&mask).expect("loc_bool");
        let e = t.elapsed().as_nanos();
        std::hint::black_box(&out);
        if e < best {
            best = e;
        }
    }

    let kept = mask.iter().filter(|&&b| b).count();
    println!("loc_bool n={n} kept={kept} iters={iters}: best={best}ns golden={golden:016x}");
}
