//! Probe/bench: Series::between on a Datetime64 column (the date-range filter idiom).
//! Datetime64 previously ERRORED (compare_non_missing_scalars_for_between had no
//! Datetime64 arm + the numeric fast path needs to_f64). Timedelta64 worked but via the
//! per-element Scalar loop. NEW = typed i64-ns fast path.
//! Run: cargo run -p fp-frame --release --example bench_between_dt -- 5000000 20

use fp_frame::Series;
use fp_index::IndexLabel;
use fp_types::Scalar;

fn best<F: FnMut()>(iters: usize, mut f: F) -> f64 {
    let mut b = u128::MAX;
    for _ in 0..iters {
        let t = std::time::Instant::now();
        f();
        b = b.min(t.elapsed().as_nanos());
    }
    b as f64 / 1e6
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let n: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(5_000_000);
    let iters: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(20);

    let base: i64 = 1_600_000_000_000_000_000;
    let labels: Vec<IndexLabel> = (0..n as i64).map(IndexLabel::Int64).collect();
    let s = Series::from_values(
        "date",
        labels,
        (0..n as i64)
            .map(|i| Scalar::Datetime64(base + (i % 10_000_000)))
            .collect(),
    )
    .unwrap();
    let lo = Scalar::Datetime64(base + 2_000_000);
    let hi = Scalar::Datetime64(base + 8_000_000);

    // Correctness: it must not error, and be exact near 1.6e18.
    match s.between(&lo, &hi, "both") {
        Ok(out) => {
            let trues = out
                .values()
                .iter()
                .filter(|v| matches!(v, Scalar::Bool(true)))
                .count();
            println!("between(Datetime64) OK — {trues} in-range of {n}");
        }
        Err(e) => {
            println!("between(Datetime64) ERRORED: {e:?}");
            return;
        }
    }
    // Exactness near 1.6e18: a 1-ns window.
    let tiny = Series::from_values(
        "t",
        vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
        vec![
            Scalar::Datetime64(base),
            Scalar::Datetime64(base + 1),
            Scalar::Datetime64(base + 2),
        ],
    )
    .unwrap();
    let r = tiny
        .between(&Scalar::Datetime64(base + 1), &Scalar::Datetime64(base + 1), "both")
        .unwrap();
    println!("exact 1-ns window = {:?} (want [F,T,F])", r.values());

    let t = best(iters, || {
        std::hint::black_box(s.between(&lo, &hi, "both").unwrap());
    });
    println!("between(Datetime64) NEW = {t:.2} ms/call ({n} rows)");
}
