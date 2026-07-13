//! Bench for `Column::describe` after making its std block collect present values
//! off the raw slice + validity — the ONLY remaining Scalar materialization in
//! describe (count/mean/quantile/min/max are already typed), so NEW describe() runs
//! without ever building the Vec<Scalar>.
//!
//! NEW = col.describe(). CONTROL = a replica of the OLD describe (generic std collect
//! over values(), same typed count/mean/quantile/min/max). Since CONTROL's values()
//! is materialized once then cached, the measured floor EXCLUDES NEW's real edge —
//! never building the 160MB@5M Scalar Vec on a fresh column. std field asserted equal.
//!
//! Run: cargo run -p fp-columnar --release --example bench_describe -- 5000000 15

use fp_columnar::{Column, ValidityMask};
use fp_types::{NullKind, Scalar};

fn old_describe(col: &Column) -> Vec<(&'static str, Scalar)> {
    let count = Scalar::Int64(col.count() as i64);
    let mean = col.mean();
    let std = {
        let nums: Vec<f64> = col
            .values()
            .iter()
            .filter(|v| !v.is_missing())
            .filter_map(|v| v.to_f64().ok())
            .collect();
        if nums.len() < 2 {
            Scalar::Null(NullKind::NaN)
        } else {
            let mu = nums.iter().sum::<f64>() / nums.len() as f64;
            let ss: f64 = nums.iter().map(|x| (x - mu).powi(2)).sum();
            Scalar::Float64((ss / (nums.len() as f64 - 1.0)).sqrt())
        }
    };
    vec![
        ("count", count),
        ("mean", mean),
        ("std", std),
        ("min", col.min()),
        ("25%", col.quantile(0.25)),
        ("50%", col.quantile(0.5)),
        ("75%", col.quantile(0.75)),
        ("max", col.max()),
    ]
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(5_000_000);
    let iters: usize = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(15);

    let idata: Vec<i64> = (0..n)
        .map(|i| {
            let h = (i as u64).wrapping_mul(2_654_435_761).wrapping_add(999);
            (h % 100_003) as i64 - 50_000
        })
        .collect();
    let fdata: Vec<f64> = idata.iter().map(|&x| x as f64 * 0.5).collect();
    let mut validity = ValidityMask::all_valid(n);
    for i in (0..n).step_by(5) {
        validity.set(i, false);
    }
    let f_all = Column::from_f64_values_owned(fdata.clone());
    let i_all = Column::from_i64_values_owned(idata.clone());
    let f_null = Column::from_f64_values_with_validity(fdata, validity.clone());
    let i_null = Column::from_i64_values_with_validity(idata, validity);

    let std_of = |d: &[(&'static str, Scalar)]| -> String {
        format!("{:?}", d.iter().find(|(k, _)| *k == "std").map(|(_, v)| v))
    };

    for (label, col) in [
        ("f64_allvalid", &f_all),
        ("i64_allvalid", &i_all),
        ("f64_nullable", &f_null),
        ("i64_nullable", &i_null),
    ] {
        let _ = col.values(); // warm the lazy Scalar-Vec cache for CONTROL

        let mut best_t = u128::MAX;
        for _ in 0..iters {
            let t = std::time::Instant::now();
            let r = col.describe().unwrap();
            best_t = best_t.min(t.elapsed().as_nanos());
            std::hint::black_box(&r);
        }
        let mut best_c = u128::MAX;
        for _ in 0..iters {
            let t = std::time::Instant::now();
            let r = old_describe(col);
            best_c = best_c.min(t.elapsed().as_nanos());
            std::hint::black_box(&r);
        }
        assert_eq!(
            std_of(&col.describe().unwrap()),
            std_of(&old_describe(col)),
            "{label}: NEW std != control std"
        );
        println!(
            "describe {label:>13} n={n} NEW={:>7.2}ms CONTROL={:>7.2}ms speedup={:.3}x",
            best_t as f64 / 1e6,
            best_c as f64 / 1e6,
            best_c as f64 / best_t as f64,
        );
    }
}
