//! Bench + golden for `Series.str().lower()` / `.upper()` on a large contiguous-Utf8
//! column — flagship text-pipeline ops. Exercises the shared `apply_str_utf8` helper's
//! contiguous-buffer path (as_utf8_contiguous in -> rolling byte buf out, zero Scalar
//! materialization; ASCII rows lowercase in place — br-frankenpandas-2krr0).
//! Golden = FNV-1a64 over the output bytes (length-prefixed per row).
//!
//! Run: cargo run -p fp-frame --example bench_str_lower --release -- 1000000 50

use std::time::Instant;

use fp_frame::Series;
use fp_index::IndexLabel;
use fp_types::Scalar;

fn build(n: usize) -> Series {
    let labels: Vec<IndexLabel> = (0..n as i64).map(IndexLabel::Int64).collect();
    let values: Vec<Scalar> = (0..n)
        .map(|row| {
            let mixed = (row as u64)
                .wrapping_mul(0x9E37_79B9_7F4A_7C15)
                .rotate_left(19);
            Scalar::Utf8(format!("Row_{:08X}_Value", mixed & 0xFFFF_FFFF))
        })
        .collect();
    Series::from_values("s", labels, values).expect("build series")
}

fn fnv1a64_update(h: &mut u64, bytes: &[u8]) {
    for b in bytes {
        *h ^= u64::from(*b);
        *h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
}

fn digest(s: &Series) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for v in s.values() {
        match v {
            Scalar::Utf8(text) => {
                fnv1a64_update(&mut h, &(text.len() as u64).to_le_bytes());
                fnv1a64_update(&mut h, text.as_bytes());
            }
            other => fnv1a64_update(&mut h, format!("{other:?}").as_bytes()),
        }
    }
    h
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let n: usize = args
        .get(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(1_000_000);
    let iters: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(50);

    let series = build(n);
    let golden_lower = digest(&series.str().lower().expect("lower"));
    let golden_upper = digest(&series.str().upper().expect("upper"));

    let mut best_lower = u128::MAX;
    let mut best_upper = u128::MAX;
    for _ in 0..iters {
        let t = Instant::now();
        let lo = series.str().lower().expect("lower");
        let e = t.elapsed().as_nanos();
        std::hint::black_box(&lo);
        if e < best_lower {
            best_lower = e;
        }

        let t = Instant::now();
        let up = series.str().upper().expect("upper");
        let e = t.elapsed().as_nanos();
        std::hint::black_box(&up);
        if e < best_upper {
            best_upper = e;
        }
    }

    println!(
        "str_case n={n} iters={iters}: lower_best={best_lower}ns upper_best={best_upper}ns \
         golden_lower={golden_lower:016x} golden_upper={golden_upper:016x}"
    );
}
