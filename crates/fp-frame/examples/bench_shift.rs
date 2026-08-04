//! Bench + golden for `Series.shift` on large Float64 and Int64 columns — a flagship
//! time-series op. Exercises the typed shift fast paths: Float64 with NaN/numeric fill
//! (br-frankenpandas-202cdf50) and Int64 with a valid Int64 fill (51601b7a), which build
//! the shifted buffer directly instead of a Vec<Scalar> + Column::new round-trip.
//! Golden = FNV-1a64 over the shifted values (NaN canonicalized) to pin output.
//!
//! Run: cargo run -p fp-frame --example bench_shift --release -- 2000000 1 50

use std::time::Instant;

use fp_frame::Series;
use fp_index::IndexLabel;
use fp_types::Scalar;

fn build_f64(n: usize) -> Series {
    let labels: Vec<IndexLabel> = (0..n as i64).map(IndexLabel::Int64).collect();
    let values: Vec<Scalar> = (0..n).map(|v| Scalar::Float64(v as f64 * 1.5)).collect();
    Series::from_values("f", labels, values).expect("build f64 series")
}

fn build_i64(n: usize) -> Series {
    let labels: Vec<IndexLabel> = (0..n as i64).map(IndexLabel::Int64).collect();
    let values: Vec<Scalar> = (0..n as i64).map(Scalar::Int64).collect();
    Series::from_values("i", labels, values).expect("build i64 series")
}

fn fnv1a64_update(h: &mut u64, bytes: &[u8]) {
    for b in bytes {
        *h ^= u64::from(*b);
        *h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
}

fn digest(s: &Series) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for val in s.values() {
        let bits: u64 = match val {
            Scalar::Float64(x) if x.is_nan() => 0x7ff8_0000_0000_0000,
            Scalar::Float64(x) => x.to_bits(),
            Scalar::Int64(x) => *x as u64,
            _ => 0xdead_beef_dead_beef, // missing/other sentinel
        };
        fnv1a64_update(&mut h, &bits.to_le_bytes());
    }
    h
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let n: usize = args
        .get(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(2_000_000);
    let periods: i64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(1);
    let iters: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(50);

    let sf = build_f64(n);
    let si = build_i64(n);
    let golden_f = digest(&sf.shift(periods).expect("shift f64"));
    let golden_i = digest(
        &si.shift_with_fill_value(periods, Scalar::Int64(0))
            .expect("shift i64"),
    );

    let mut best_f = u128::MAX;
    let mut best_i = u128::MAX;
    for _ in 0..iters {
        let t = Instant::now();
        let a = sf.shift(periods).expect("shift f64");
        let e = t.elapsed().as_nanos();
        std::hint::black_box(&a);
        if e < best_f {
            best_f = e;
        }

        let t = Instant::now();
        let b = si
            .shift_with_fill_value(periods, Scalar::Int64(0))
            .expect("shift i64");
        let e = t.elapsed().as_nanos();
        std::hint::black_box(&b);
        if e < best_i {
            best_i = e;
        }
    }

    println!(
        "shift n={n} periods={periods} iters={iters}: f64_best={best_f}ns i64_best={best_i}ns \
         golden_f={golden_f:016x} golden_i={golden_i:016x}"
    );
}
