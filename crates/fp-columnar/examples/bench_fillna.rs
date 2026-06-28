//! Column::fillna over a 5M nullable column (1/4 missing). bench_fillna <n> <dt>
use fp_columnar::Column;
use fp_columnar::ValidityMask;
use fp_types::Scalar;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(5_000_000);
    let dt = a.get(2).map(String::as_str).unwrap_or("i64");
    let mut validity = ValidityMask::all_valid(n);
    for i in (0..n).step_by(4) {
        validity.set(i, false);
    }
    let (col, fill) = if dt == "f64" {
        (
            Column::from_f64_values_with_validity((0..n).map(|i| i as f64).collect(), validity),
            Scalar::Float64(0.0),
        )
    } else {
        (
            Column::from_i64_values_with_validity((0..n as i64).collect(), validity),
            Scalar::Int64(0),
        )
    };
    let mut best = u128::MAX;
    for _ in 0..6 {
        let t = std::time::Instant::now();
        let r = col.fillna(&fill).unwrap();
        std::hint::black_box(r.len());
        let e = t.elapsed().as_nanos();
        if e < best {
            best = e;
        }
    }
    println!("fillna {dt} n={n}: best={best}ns ({:.2}ms)", best as f64 / 1e6);
}
