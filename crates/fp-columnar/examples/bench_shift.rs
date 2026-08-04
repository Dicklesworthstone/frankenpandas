//! Column::shift over a 5M column. bench_shift <n> <dt> <fill>
use fp_columnar::Column;
use fp_types::Scalar;
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(5_000_000);
    let dt = a.get(2).map(String::as_str).unwrap_or("f64");
    let fill_mode = a.get(3).map(String::as_str).unwrap_or("null");
    let col = if dt == "i64" {
        Column::from_i64_values((0..n as i64).map(|i| i % 100000).collect())
    } else if dt == "bool" {
        Column::from_bool_values((0..n).map(|i| (i & 1) == 0).collect())
    } else {
        Column::from_f64_values((0..n).map(|i| i as f64 * 0.5).collect())
    };
    let fill = if fill_mode == "zero" {
        if dt == "i64" {
            Scalar::Int64(0)
        } else if dt == "bool" {
            Scalar::Bool(false)
        } else {
            Scalar::Float64(0.0)
        }
    } else {
        Scalar::Null(fp_types::NullKind::NaN)
    };
    let mut best = u128::MAX;
    for _ in 0..6 {
        let t = std::time::Instant::now();
        let r = col.shift(1, fill.clone()).unwrap();
        std::hint::black_box(r.len());
        let e = t.elapsed().as_nanos();
        if e < best {
            best = e;
        }
    }
    println!(
        "shift {dt} fill={fill_mode} n={n}: best={best}ns ({:.2}ms)",
        best as f64 / 1e6
    );
}
