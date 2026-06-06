//! Bench + golden for Column::mode — dense direct-address Int64 tally.
//!
//! Run: cargo run -p fp-columnar --example bench_mode --release -- [bench|golden]
//!
//! mode() tallied via a SipHash `HashMap<Key, (count, &Scalar)>` then sorted the
//! winners. For an all-valid bounded-range Int64 column a dense direct-address
//! histogram tallies in O(n) with no hashing, and the winners come out already
//! ascending (slot order) — no sort. Bit-identical.

use fp_columnar::Column;
use fp_types::Scalar;
use std::time::Instant;

/// splitmix64 — deterministic, no rand dependency.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

fn build(n: usize, range: i64) -> Column {
    let mut rng = Rng(0xDEAD_BEEF_1234_5678);
    let mut data = Vec::with_capacity(n);
    for _ in 0..n {
        data.push((rng.next() % range as u64) as i64);
    }
    // Force a couple of clear modal values so the winners list is non-trivial.
    for _ in 0..(n / 50) {
        data.push(7);
        data.push(range - 3);
    }
    Column::from_i64_values(data)
}

fn dump(col: &Column) -> String {
    let mut s = String::new();
    for v in col.values() {
        match v {
            Scalar::Int64(i) => s.push_str(&format!("{i},")),
            other => s.push_str(&format!("{other:?},")),
        }
    }
    s
}

fn main() {
    let mode = std::env::args().nth(1).unwrap_or_else(|| "bench".to_string());

    if mode == "golden" {
        // A spread of sizes/ranges, including ties, to certify equivalence.
        let mut out = String::new();
        for (n, range) in [(1000usize, 17i64), (5000, 251), (20000, 1024)] {
            let col = build(n, range);
            out.push_str(&format!("n={n},range={range}:{}\n", dump(&col.mode().unwrap())));
        }
        print!("{out}");
        return;
    }

    let n: usize = 4_000_000;
    let range: i64 = 50_000;
    let col = build(n, range);

    // warmup
    let _ = col.mode().unwrap();

    let iters = 20;
    let mut best = f64::INFINITY;
    for _ in 0..iters {
        let t0 = Instant::now();
        let m = col.mode().unwrap();
        let d = t0.elapsed().as_secs_f64();
        std::hint::black_box(&m);
        if d < best {
            best = d;
        }
    }
    println!("mode n={n} range={range} best={:.3}ms", best * 1e3);
}
