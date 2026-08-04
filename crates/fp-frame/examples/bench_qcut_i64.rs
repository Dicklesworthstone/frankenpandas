//! Bench for `qcut` on an all-valid Int64 Series after adding the typed i64 input
//! path (as_i64_slice ⇒ Some(v as f64)), which skips the `.values()` Scalar
//! materialization the generic arm forces.
//!
//! Public example API can't inject a pre-built Column, so we build the Series via
//! Series::from_values (Int64 Scalars) and confirm it exposes a typed backing
//! (as_i64_slice ⇒ Some) so the NEW arm fires. We report qcut(typed) total time
//! and, separately, the cost of materializing the i64 column's Scalar Vec — the
//! input work the OLD generic arm did (via `.values()`) that NEW now skips.
//!
//! Run: cargo run -p fp-frame --release --example bench_qcut_i64 -- 5000000 6

use fp_columnar::Column;
use fp_frame::{qcut, Series};
use fp_index::IndexLabel;
use fp_types::Scalar;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(5_000_000);
    let iters: usize = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(6);
    let q: usize = 10;

    let ivals: Vec<i64> = (0..n)
        .map(|i| {
            let h = (i as u64).wrapping_mul(2_654_435_761).wrapping_add(999);
            (h % 1_000_003) as i64 - 500_000
        })
        .collect();
    let idx: Vec<IndexLabel> = (0..n as i64).map(IndexLabel::Int64).collect();
    let scalars: Vec<Scalar> = ivals.iter().map(|&v| Scalar::Int64(v)).collect();

    let ser = Series::from_values("x", idx, scalars).unwrap();
    let typed = ser.column().as_i64_slice().is_some();

    // qcut(typed i64) — uses the NEW typed arm when `typed`.
    let mut t_qcut = u128::MAX;
    for _ in 0..iters {
        let t = std::time::Instant::now();
        let r = qcut(&ser, q).unwrap();
        t_qcut = t_qcut.min(t.elapsed().as_nanos());
        std::hint::black_box(r.len());
    }

    // Cost the OLD generic arm paid on input: materialize an i64 column's Scalar
    // Vec (fresh column each iter so the OnceLock cache doesn't hide it). This is
    // exactly the `.values()` work NEW's as_i64_slice arm skips.
    let mut t_mat = u128::MAX;
    for _ in 0..iters {
        let col = Column::from_i64_values_owned(ivals.clone());
        let t = std::time::Instant::now();
        let vals = col.values();
        t_mat = t_mat.min(t.elapsed().as_nanos());
        std::hint::black_box(vals.len());
    }

    println!(
        "qcut_i64 n={n} q={q} typed={typed} qcut_total={:.2}ms scalar_materialize_input={:.2}ms (~cost NEW skips)",
        t_qcut as f64 / 1e6,
        t_mat as f64 / 1e6,
    );
}
