//! Third gauntlet sweep: argsort, duplicated, unique, between. 2M Float64.
//! argsort is Scalar-based (compare_scalars_with_na_last over values()) — typed candidate.
//!
//! Run: cargo run -p fp-frame --example bench_misc3 --release -- 2000000 30

use std::time::Instant;

use fp_frame::Series;
use fp_index::IndexLabel;
use fp_types::Scalar;

fn best<F: FnMut()>(iters: usize, mut f: F) -> u128 {
    let mut b = u128::MAX;
    for _ in 0..iters {
        let t = Instant::now();
        f();
        let e = t.elapsed().as_nanos();
        if e < b {
            b = e;
        }
    }
    b
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let n: usize = args
        .get(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(2_000_000);
    let iters: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(30);
    let labels: Vec<IndexLabel> = (0..n as i64).map(IndexLabel::Int64).collect();
    let mut st: u64 = 0x1234_5678_9abc_def1;
    let mut nextf = || {
        st = st.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        ((st >> 11) as f64) / (1u64 << 53) as f64
    };
    // ~1000 distinct buckets so duplicated/unique do real hashing work.
    let vals: Vec<Scalar> = (0..n)
        .map(|_| Scalar::Float64((nextf() * 1000.0).floor()))
        .collect();
    let s = Series::from_values("v", labels, vals).unwrap();

    let argsort = best(iters, || {
        std::hint::black_box(s.argsort(true).expect("argsort"));
    });
    let duplicated = best(iters, || {
        std::hint::black_box(s.duplicated().expect("duplicated"));
    });
    let unique = best(iters, || {
        std::hint::black_box(s.unique());
    });
    let between = best(iters, || {
        std::hint::black_box(
            s.between(&Scalar::Float64(100.0), &Scalar::Float64(900.0), "both")
                .expect("between"),
        );
    });

    println!(
        "misc3 n={n}: argsort={argsort}ns duplicated={duplicated}ns unique={unique}ns between={between}ns"
    );
}
