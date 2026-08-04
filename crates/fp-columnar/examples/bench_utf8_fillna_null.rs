//! Bench for `Column::fillna` on a NULLABLE contiguous-Utf8 column (`LazyNullableUtf8`) after
//! adding the typed Utf8 arm — builds the filled output as one contiguous buffer (span for
//! present rows, fill bytes for missing), instead of the generic loop that clones a heap
//! String per present row + the fill per missing row.
//!
//! NEW = col.fillna(&fill) [typed arm]. COLD = materialize + per-row Scalar clone-fill.
//! WARM = same over a pre-materialized Vec (no input materialization).
//!
//! Run: cargo run -p fp-columnar --release --example bench_utf8_fillna_null -- 5000000 20

use fp_columnar::{Column, ValidityMask};
use fp_types::{DType, NullKind, Scalar};

fn materialize(bytes: &[u8], offsets: &[usize], present: &[bool]) -> Vec<Scalar> {
    offsets
        .windows(2)
        .enumerate()
        .map(|(i, w)| {
            if present[i] {
                Scalar::Utf8(std::str::from_utf8(&bytes[w[0]..w[1]]).unwrap().to_string())
            } else {
                Scalar::Null(NullKind::Null)
            }
        })
        .collect()
}

fn fill_clone(vals: &[Scalar], fill: &Scalar) -> Column {
    let out: Vec<Scalar> = vals
        .iter()
        .map(|v| if v.is_missing() { fill.clone() } else { v.clone() })
        .collect();
    Column::new(DType::Utf8, out).unwrap()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let n: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(5_000_000);
    let iters: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(20);

    let mut bytes: Vec<u8> = Vec::with_capacity(n * 10);
    let mut offsets: Vec<usize> = Vec::with_capacity(n + 1);
    offsets.push(0);
    let mut validity = ValidityMask::all_valid(n);
    let mut present: Vec<bool> = Vec::with_capacity(n);
    for i in 0..n {
        if i % 5 == 0 {
            validity.set(i, false);
            present.push(false);
        } else {
            bytes.extend_from_slice(format!("cat_{:06}", i % 1000).as_bytes());
            present.push(true);
        }
        offsets.push(bytes.len());
    }
    let col = Column::from_utf8_values_with_validity(bytes.clone(), offsets.clone(), validity);
    let fill = Scalar::Utf8("Unknown".to_string());
    let warm_vals = materialize(&bytes, &offsets, &present);

    let mut best_t = u128::MAX;
    for _ in 0..iters {
        let t = std::time::Instant::now();
        let r = col.fillna(&fill).unwrap();
        best_t = best_t.min(t.elapsed().as_nanos());
        std::hint::black_box(&r);
    }
    let mut best_cold = u128::MAX;
    for _ in 0..iters {
        let t = std::time::Instant::now();
        let vals = materialize(&bytes, &offsets, &present);
        let r = fill_clone(&vals, &fill);
        best_cold = best_cold.min(t.elapsed().as_nanos());
        std::hint::black_box(&r);
    }
    let mut best_warm = u128::MAX;
    for _ in 0..iters {
        let t = std::time::Instant::now();
        let r = fill_clone(&warm_vals, &fill);
        best_warm = best_warm.min(t.elapsed().as_nanos());
        std::hint::black_box(&r);
    }
    let got = col.fillna(&fill).unwrap();
    let want = fill_clone(&warm_vals, &fill);
    let gv = got.values();
    let wv = want.values();
    assert_eq!(gv.len(), wv.len());
    for k in 0..n {
        assert_eq!(format!("{:?}", gv.get(k)), format!("{:?}", wv.get(k)), "slot {k}");
    }
    println!(
        "fillna utf8_nullable n={n} NEW={:>7.2}ms COLD={:>7.2}ms(={:.2}x) WARM={:>7.2}ms(={:.2}x)",
        best_t as f64 / 1e6,
        best_cold as f64 / 1e6,
        best_cold as f64 / best_t as f64,
        best_warm as f64 / 1e6,
        best_warm as f64 / best_t as f64,
    );
}
