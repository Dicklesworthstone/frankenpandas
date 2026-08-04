//! Series.combine_first head-to-head. 2M Float64 same index, self ~50% NaN, other fills.
//! Run: cargo run -p fp-frame --example bench_combine_cc --release -- 2000000 30 [construct|materialize|values]

use std::time::Instant;

use fp_frame::Series;
use fp_index::IndexLabel;
use fp_types::Scalar;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let n: usize = args
        .get(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(2_000_000);
    let iters: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(30);
    let mode = args.get(3).map(String::as_str).unwrap_or("construct");
    let labels: Vec<IndexLabel> = (0..n as i64).map(IndexLabel::Int64).collect();
    let a = Series::from_values(
        "a",
        labels.clone(),
        (0..n)
            .map(|i| {
                if i % 2 == 0 {
                    Scalar::Float64(i as f64)
                } else {
                    Scalar::Float64(f64::NAN)
                }
            })
            .collect(),
    )
    .unwrap();
    let b = Series::from_values(
        "b",
        labels,
        (0..n).map(|i| Scalar::Float64(i as f64 * 2.0)).collect(),
    )
    .unwrap();

    let mut best = u128::MAX;
    for _ in 0..iters {
        let t = Instant::now();
        let out = a.combine_first(&b).expect("combine_first");
        match mode {
            "construct" => {
                std::hint::black_box(&out);
            }
            "materialize" => {
                std::hint::black_box(out.column().as_f64_slice().expect("f64"));
            }
            "values" => {
                std::hint::black_box(out.values());
            }
            other => panic!("unknown mode: {other}"),
        };
        let e = t.elapsed().as_nanos();
        if e < best {
            best = e;
        }
    }
    println!("combine_first mode={mode} n={n}: best={best}ns");
}
