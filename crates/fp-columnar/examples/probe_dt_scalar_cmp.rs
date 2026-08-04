//! Probe: does `Column::compare_scalar` on a Datetime64/Timedelta64 column vs a
//! temporal scalar error, work, or lose precision? And how slow?
//! Run: cargo run -p fp-columnar --release --example probe_dt_scalar_cmp -- 5000000 20

use fp_columnar::{Column, ComparisonOp};
use fp_types::Scalar;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let n: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(5_000_000);
    let iters: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(20);

    let base: i64 = 1_600_000_000_000_000_000;
    let mut state: u64 = 0x51CA_1AA5;
    let data: Vec<i64> = (0..n)
        .map(|_| {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
            base + ((state >> 30) % 1_000_000) as i64
        })
        .collect();
    let col = Column::from_datetime64_values(data.clone());
    let threshold = Scalar::Datetime64(base + 500_000);

    match col.compare_scalar(&threshold, ComparisonOp::Gt) {
        Ok(out) => println!("compare_scalar(Datetime64 > Datetime64) OK, len={}", out.len()),
        Err(e) => {
            println!("compare_scalar(Datetime64 > Datetime64) ERRORED: {e:?}");
        }
    }

    // Exactness near 1.6e18.
    let small = Column::from_datetime64_values(vec![base, base + 1, base + 2]);
    match small.compare_scalar(&Scalar::Datetime64(base + 1), ComparisonOp::Lt) {
        Ok(out) => println!("near-1.6e18  col < base+1 = {:?} (want [t,f,f])", out.values()),
        Err(e) => println!("near-1.6e18 ERRORED: {e:?}"),
    }

    // Timing (only if it works).
    if col.compare_scalar(&threshold, ComparisonOp::Gt).is_ok() {
        let mut best = u128::MAX;
        for _ in 0..iters {
            let t = std::time::Instant::now();
            let out = col.compare_scalar(&threshold, ComparisonOp::Gt).unwrap();
            best = best.min(t.elapsed().as_nanos());
            std::hint::black_box(&out);
        }
        println!("timing: {:.2}ms/call ({n} rows)", best as f64 / 1e6);
    }
}
