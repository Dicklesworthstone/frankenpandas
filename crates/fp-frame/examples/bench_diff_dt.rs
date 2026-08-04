//! Probe/bench: Series::diff on a Datetime64 column (time between consecutive events).
//! Datetime64 diff previously SILENTLY returned all-null (generic to_f64 cascade caught
//! the temporal Err and emitted Null(NaN)). NEW = typed i64-ns -> Timedelta64.
//! Run: cargo run -p fp-frame --release --example bench_diff_dt -- 5000000 20

use fp_frame::Series;
use fp_index::IndexLabel;
use fp_types::{DType, Scalar};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let n: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(5_000_000);
    let iters: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(20);

    let base: i64 = 1_600_000_000_000_000_000;
    let labels: Vec<IndexLabel> = (0..n as i64).map(IndexLabel::Int64).collect();
    // Monotonic timestamps 60s apart (in ns).
    let s = Series::from_values(
        "ts",
        labels,
        (0..n as i64)
            .map(|i| Scalar::Datetime64(base + i * 60_000_000_000))
            .collect(),
    )
    .unwrap();

    let out = s.diff(1).unwrap();
    println!(
        "diff(Datetime64) -> dtype={:?} (want Timedelta64)",
        out.dtype()
    );
    // First is NaT (no predecessor); rest should be 60s = 60_000_000_000 ns.
    let vals = out.values();
    println!(
        "first={:?} (want NaT/Null), second={:?} (want Timedelta64(60_000_000_000))",
        vals[0], vals[1]
    );
    let non_null_60s = vals
        .iter()
        .filter(|v| matches!(v, Scalar::Timedelta64(60_000_000_000)))
        .count();
    println!("rows == 60s: {non_null_60s} of {n} (want {})", n - 1);
    assert_eq!(out.dtype(), DType::Timedelta64);

    // Exactness near 1.6e18: 1-ns steps.
    let tiny = Series::from_values(
        "t",
        vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
        vec![
            Scalar::Datetime64(base),
            Scalar::Datetime64(base + 1),
            Scalar::Datetime64(base + 3),
        ],
    )
    .unwrap();
    println!("1-ns diffs = {:?} (want [NaT, Td 1, Td 2])", tiny.diff(1).unwrap().values());

    let mut best = u128::MAX;
    for _ in 0..iters {
        let t = std::time::Instant::now();
        let o = s.diff(1).unwrap();
        best = best.min(t.elapsed().as_nanos());
        std::hint::black_box(&o);
    }
    println!("diff(Datetime64) NEW = {:.2} ms/call ({n} rows)", best as f64 / 1e6);
}
