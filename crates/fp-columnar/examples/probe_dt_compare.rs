//! Probe: does `Column::binary_comparison` on two Datetime64 columns error, work,
//! or lose precision? And how slow is it vs a typed i64-ns compare?
//! Run: cargo run -p fp-columnar --release --example probe_dt_compare -- 5000000 20

use fp_columnar::{Column, ComparisonOp};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let n: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(5_000_000);
    let iters: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(20);

    // Realistic epoch-ns near 1.6e18 (>> 2^53), with small deltas.
    let base: i64 = 1_600_000_000_000_000_000;
    let mut state: u64 = 0x1234_9E37;
    let la: Vec<i64> = (0..n)
        .map(|_| {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
            base + ((state >> 30) % 1_000_000) as i64
        })
        .collect();
    let ra: Vec<i64> = (0..n)
        .map(|_| {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
            base + ((state >> 30) % 1_000_000) as i64
        })
        .collect();

    let lcol = Column::from_datetime64_values(la.clone());
    let rcol = Column::from_datetime64_values(ra.clone());

    // 1) Does it even work?
    match lcol.binary_comparison(&rcol, ComparisonOp::Lt) {
        Ok(out) => println!("binary_comparison(Lt) OK, out.len()={}", out.len()),
        Err(e) => {
            println!("binary_comparison(Lt) ERRORED: {e:?}");
            return;
        }
    }

    // 2) Correctness near 1.6e18: two timestamps differing by 1 ns.
    let a = Column::from_datetime64_values(vec![base, base + 1, base + 2]);
    let b = Column::from_datetime64_values(vec![base + 1, base + 1, base + 1]);
    let lt = a.binary_comparison(&b, ComparisonOp::Lt).unwrap();
    let eq = a.binary_comparison(&b, ComparisonOp::Eq).unwrap();
    println!("near-1.6e18  a<b  = {:?}  (exact want [true,false,false])", lt.values());
    println!("near-1.6e18  a==b = {:?}  (exact want [false,true,false])", eq.values());

    // 3) Timing: current binary_comparison vs a hand-rolled typed i64-ns compare.
    let mut best_cur = u128::MAX;
    for _ in 0..iters {
        let t = std::time::Instant::now();
        let out = lcol.binary_comparison(&rcol, ComparisonOp::Lt).unwrap();
        best_cur = best_cur.min(t.elapsed().as_nanos());
        std::hint::black_box(&out);
    }
    let mut best_typed = u128::MAX;
    for _ in 0..iters {
        let t = std::time::Instant::now();
        let bools: Vec<bool> = la.iter().zip(&ra).map(|(&x, &y)| x < y).collect();
        let out = Column::from_bool_values(bools);
        best_typed = best_typed.min(t.elapsed().as_nanos());
        std::hint::black_box(&out);
    }
    println!(
        "n={n}  CURRENT={:.2}ms  TYPED_i64={:.2}ms  ({:.1}x)",
        best_cur as f64 / 1e6,
        best_typed as f64 / 1e6,
        best_cur as f64 / best_typed as f64,
    );
}
