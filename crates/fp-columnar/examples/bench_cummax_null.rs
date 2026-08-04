//! Bench for `Column::cummax`/`cummin` on a nullable Int64/Float64 column after
//! adding the typed nullable path (raw slice + validity fold, skipping the
//! nancummax-over-Vec<Scalar> materialization).
//!
//! NEW = col.cummax() (typed arm). CONTROL = the old path: nancummax over the
//! materialized values() + rebuild a column from those Scalars. The source
//! values() is cached, so the control's one-time Scalar materialization is
//! excluded ⇒ a CONSERVATIVE lower bound (NEW also avoids that materialization on
//! a fresh column). Checksums asserted equal.
//!
//! Run: cargo run -p fp-columnar --release --example bench_cummax_null -- 5000000 15

use fp_columnar::{Column, ValidityMask};
use fp_types::Scalar;

fn fold(col: &Column) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for v in col.values() {
        let bits = match v {
            Scalar::Float64(x) => x.to_bits(),
            Scalar::Null(_) => 0xFFFF_FFFF_FFFF_FFFF,
            _ => 0xDEAD,
        };
        h ^= bits;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(5_000_000);
    let iters: usize = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(15);

    let idata: Vec<i64> = (0..n)
        .map(|i| {
            let h = (i as u64).wrapping_mul(2_654_435_761).wrapping_add(999);
            (h % 1_000_003) as i64 - 500_000
        })
        .collect();
    let fdata: Vec<f64> = idata.iter().map(|&x| x as f64 * 0.5).collect();
    let mut validity = ValidityMask::all_valid(n);
    for i in (0..n).step_by(5) {
        validity.set(i, false);
    }
    let icol = Column::from_i64_values_with_validity(idata, validity.clone());
    let fcol = Column::from_f64_values_with_validity(fdata, validity);

    for (label, col) in [("i64", &icol), ("f64", &fcol)] {
        // TREATMENT
        let mut best_t = u128::MAX;
        let mut ck_t: u64 = 0;
        for _ in 0..iters {
            let t = std::time::Instant::now();
            let out = col.cummax().expect("cummax");
            best_t = best_t.min(t.elapsed().as_nanos());
            ck_t = ck_t.wrapping_add(fold(&out));
        }
        // CONTROL (old path: nancummax over materialized Scalars + rebuild).
        let mut best_c = u128::MAX;
        let mut ck_c: u64 = 0;
        for _ in 0..iters {
            let t = std::time::Instant::now();
            let scalars = fp_types::nancummax(col.values());
            let out = Column::from_values(scalars).expect("col");
            best_c = best_c.min(t.elapsed().as_nanos());
            ck_c = ck_c.wrapping_add(fold(&out));
        }
        assert_eq!(ck_t, ck_c, "{label}: treatment != control");
        println!(
            "cummax_null {label} n={n} iters={iters} NEW={:.2}ms CONTROL={:.2}ms speedup={:.3}x",
            best_t as f64 / 1e6,
            best_c as f64 / 1e6,
            best_c as f64 / best_t as f64,
        );
    }
}
