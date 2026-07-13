//! Bench for `Column::searchsorted_values_with_sorter` on a large all-valid Int64
//! column after adding the typed indirect binary search (partition over
//! data[sorter[mid]] typed, instead of the Scalar core materializing the whole
//! Vec<Scalar> and running compare_scalars_na_last per step for every needle).
//!
//! NEW = col.searchsorted_values_with_sorter (typed). CONTROL = a manual binary
//! search over the pre-materialized col.values() (Scalar access + match) with the
//! SAME sorter — a proxy for the old Scalar core (understates it, since the real
//! core also runs compare_scalars_na_last, so the speedup is a LOWER bound; and
//! NEW additionally never materializes the Scalar Vec at all). Positions asserted
//! equal.
//!
//! Run: cargo run -p fp-columnar --release --example bench_searchsorted_sorter -- 5000000 200000 8

use fp_columnar::Column;
use fp_types::Scalar;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(5_000_000);
    let m: usize = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(200_000);
    let iters: usize = a.get(3).and_then(|s| s.parse().ok()).unwrap_or(8);

    // Unsorted wide-ish Int64; sorter = argsort (so the physical column is NOT
    // sorted — the realistic searchsorted(sorter=..) shape).
    let data: Vec<i64> = (0..n)
        .map(|i| {
            let h = (i as u64).wrapping_mul(2_654_435_761).wrapping_add(999);
            (h % 4_000_003) as i64 - 2_000_000
        })
        .collect();
    let col = Column::from_i64_values(data);
    let sorter = col.argsort();
    let needles: Vec<Scalar> = (0..m)
        .map(|j| {
            let h = (j as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
            Scalar::Int64((h % 4_000_003) as i64 - 2_000_000)
        })
        .collect();

    // Force the Scalar Vec once so the control's one-time materialization is
    // excluded from its timing (isolating the per-comparison cost).
    let _ = col.values().len();

    let new_pos = |c: &Column| -> Vec<i64> {
        c.searchsorted_values_with_sorter(&needles, "left", &sorter)
            .expect("ss")
            .values()
            .iter()
            .map(|v| match v {
                Scalar::Int64(x) => *x,
                _ => 0,
            })
            .collect()
    };
    let ctl_pos = |c: &Column| -> Vec<i64> {
        let vals = c.values();
        needles
            .iter()
            .map(|nd| {
                let nv = match nd {
                    Scalar::Int64(v) => *v,
                    _ => 0,
                };
                let (mut lo, mut hi) = (0usize, sorter.len());
                while lo < hi {
                    let mid = lo + (hi - lo) / 2;
                    let mv = match &vals[sorter[mid]] {
                        Scalar::Int64(x) => *x,
                        _ => i64::MAX,
                    };
                    if mv < nv {
                        lo = mid + 1;
                    } else {
                        hi = mid;
                    }
                }
                lo as i64
            })
            .collect()
    };
    assert_eq!(new_pos(&col), ctl_pos(&col), "NEW != control positions");

    let mut best_t = u128::MAX;
    for _ in 0..iters {
        let t = std::time::Instant::now();
        let r = new_pos(&col);
        best_t = best_t.min(t.elapsed().as_nanos());
        std::hint::black_box(r.len());
    }
    let mut best_c = u128::MAX;
    for _ in 0..iters {
        let t = std::time::Instant::now();
        let r = ctl_pos(&col);
        best_c = best_c.min(t.elapsed().as_nanos());
        std::hint::black_box(r.len());
    }
    println!(
        "searchsorted_sorter n={n} m={m} iters={iters} NEW(typed)={:.2}ms CONTROL(scalar)={:.2}ms speedup={:.3}x",
        best_t as f64 / 1e6,
        best_c as f64 / 1e6,
        best_c as f64 / best_t as f64,
    );
}
