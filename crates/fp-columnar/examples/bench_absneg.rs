//! Column abs/neg over 5M numeric values. bench_absneg <n> <op> <dtype>
use fp_columnar::{Column, ValidityMask};

fn nullable_f64_column(n: usize) -> Column {
    let data: Vec<f64> = (0..n as i64).map(|i| (i % 1000) as f64 - 500.0).collect();
    let mut validity = ValidityMask::all_valid(n);
    for i in (0..n).step_by(5) {
        validity.set(i, false);
    }
    Column::from_f64_values_with_validity(data, validity)
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(5_000_000);
    let op = a.get(2).map(String::as_str).unwrap_or("abs");
    let dtype = a.get(3).map(String::as_str).unwrap_or("i64");
    let col = match dtype {
        "f64-null" | "nullable-f64" => nullable_f64_column(n),
        "f64" => {
            Column::from_f64_values((0..n as i64).map(|i| (i % 1000) as f64 - 500.0).collect())
        }
        _ => Column::from_i64_values((0..n as i64).map(|i| (i % 1000) - 500).collect()),
    };
    let mut best = u128::MAX;
    for _ in 0..6 {
        let t = std::time::Instant::now();
        let r = if op == "neg" {
            col.neg().unwrap()
        } else {
            col.abs().unwrap()
        };
        std::hint::black_box(r.len());
        best = best.min(t.elapsed().as_nanos());
    }
    println!(
        "{op} {dtype} n={n}: best={best}ns ({:.2}ms)",
        best as f64 / 1e6
    );
}
