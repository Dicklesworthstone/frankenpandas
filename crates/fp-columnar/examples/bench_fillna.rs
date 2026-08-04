//! Column::fillna over a 5M nullable column (1/4 missing). bench_fillna <n> <dt> [iters]
use fp_columnar::{Column, ValidityMask};
use fp_types::{Scalar, cast_scalar};

fn scalar_reference(col: &Column, fill: &Scalar) -> Column {
    let cast_fill = cast_scalar(fill, col.dtype()).unwrap();
    let values = col
        .values()
        .iter()
        .map(|value| {
            if value.is_missing() {
                cast_fill.clone()
            } else {
                value.clone()
            }
        })
        .collect();
    Column::new(col.dtype(), values).unwrap()
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(5_000_000);
    let dt = a.get(2).map(String::as_str).unwrap_or("i64");
    let iters: usize = a.get(3).and_then(|s| s.parse().ok()).unwrap_or(9).max(1);
    let mut validity = ValidityMask::all_valid(n);
    for i in (0..n).step_by(4) {
        validity.set(i, false);
    }
    let (col, fill) = match dt {
        "f64" => (
            Column::from_f64_values_with_validity((0..n).map(|i| i as f64).collect(), validity),
            Scalar::Float64(0.0),
        ),
        "bool" => (
            Column::from_bool_values_with_validity((0..n).map(|i| i & 1 == 0).collect(), validity),
            Scalar::Bool(true),
        ),
        _ => (
            Column::from_i64_values_with_validity((0..n as i64).collect(), validity),
            Scalar::Int64(0),
        ),
    };

    let reference = scalar_reference(&col, &fill);
    let candidate = col.fillna(&fill).unwrap();
    assert_eq!(candidate.dtype(), reference.dtype());
    assert_eq!(candidate.len(), reference.len());
    assert_eq!(
        candidate.validity().count_valid(),
        reference.validity().count_valid()
    );
    assert_eq!(candidate.values(), reference.values());

    let mut reference_ns = Vec::with_capacity(iters);
    let mut candidate_ns = Vec::with_capacity(iters);
    for pass in 0..iters {
        if pass & 1 == 0 {
            let t = std::time::Instant::now();
            std::hint::black_box(scalar_reference(&col, &fill));
            reference_ns.push(t.elapsed().as_nanos());
            let t = std::time::Instant::now();
            std::hint::black_box(col.fillna(&fill).unwrap());
            candidate_ns.push(t.elapsed().as_nanos());
        } else {
            let t = std::time::Instant::now();
            std::hint::black_box(col.fillna(&fill).unwrap());
            candidate_ns.push(t.elapsed().as_nanos());
            let t = std::time::Instant::now();
            std::hint::black_box(scalar_reference(&col, &fill));
            reference_ns.push(t.elapsed().as_nanos());
        }
    }
    reference_ns.sort_unstable();
    candidate_ns.sort_unstable();
    let p50 = iters / 2;
    let p95 = (iters * 95 / 100).min(iters - 1);
    println!(
        "fillna {dt} reference n={n}: p50={}ns p95={}ns best={}ns",
        reference_ns[p50], reference_ns[p95], reference_ns[0]
    );
    println!(
        "fillna {dt} candidate n={n}: p50={}ns p95={}ns best={}ns",
        candidate_ns[p50], candidate_ns[p95], candidate_ns[0]
    );
}
