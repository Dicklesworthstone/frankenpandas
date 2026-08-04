//! Bench + golden for the all-valid Int64 counting-sort `Column::rank` fast
//! path (the kernel behind groupby_rank on integer values). A bounded-range
//! i64 column of length `n` is ranked `iters` times. Golden = FNV-1a64 over the
//! f64 rank bits + per-row validity.
//!
//! Run: cargo run -p fp-columnar --example bench_rank_i64 --release -- 200000 200 average

use std::time::Instant;

use fp_columnar::Column;

fn build(n: usize, groups: i64) -> Column {
    // Bounded-range i64 (hits the counting-sort path) with realistic ties.
    let v: Vec<i64> = (0..n as i64)
        .map(|i| (i.wrapping_mul(2_654_435_761)).rem_euclid(groups))
        .collect();
    Column::from_i64_values(v)
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

fn digest(col: &Column) -> u64 {
    let n = col.len();
    let mut buf = Vec::with_capacity(n * 9);
    for i in 0..n {
        let (bits, valid) = match col.value(i) {
            Some(fp_types::Scalar::Float64(f)) => (f.to_bits(), 1u8),
            _ => (0u64, 0u8),
        };
        buf.extend_from_slice(&bits.to_le_bytes());
        buf.push(valid);
    }
    fnv1a64(&buf)
}

fn methods() -> [&'static str; 5] {
    ["average", "min", "max", "first", "dense"]
}

fn main() {
    let mut args = std::env::args().skip(1);
    let n: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(200_000);
    let iters: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(200);
    let which = args.next().unwrap_or_else(|| "average".to_string());

    let col = build(n, 100);

    for m in methods() {
        for asc in [true, false] {
            let out = col.rank(m, asc).unwrap();
            println!("GOLDEN method={m} asc={asc} fnv={:016x}", digest(&out));
        }
    }
    if which == "all" {
        return;
    }

    let _ = col.rank(&which, true).unwrap();
    let t = Instant::now();
    let mut sink = 0u64;
    for _ in 0..iters {
        let out = col.rank(&which, true).unwrap();
        sink = sink.wrapping_add(out.len() as u64);
    }
    let d = t.elapsed();
    std::hint::black_box(sink);
    println!(
        "TIMING method={which} n={n} iters={iters} per_iter={:.4}ms",
        d.as_secs_f64() * 1e3 / iters as f64
    );
}
