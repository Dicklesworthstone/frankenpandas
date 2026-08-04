//! Bench for `Column::replace_values` on a NULLABLE contiguous-Utf8 column
//! (`LazyNullableUtf8`) after adding the typed arm — O(1) map probe + contiguous build,
//! instead of the generic loop that materializes `Vec<Scalar::Utf8>` AND does an O(k) per-row
//! linear `semantic_eq` scan over targets (O(N·k)) + a per-row Scalar clone.
//!
//! NEW = col.replace_values(&t, &r) [typed arm]. COLD = materialize + O(N·k) scan + clone.
//! WARM = same over a pre-materialized Vec (no input materialization).
//!
//! Run: cargo run -p fp-columnar --release --example bench_utf8_replace_null -- 5000000 20

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

// Generic replace replica: materialize (via `vals`) already done; per-row O(k) linear scan.
fn replace_scan(vals: &[Scalar], targets: &[String], repls: &[String]) -> Column {
    let out: Vec<Scalar> = vals
        .iter()
        .map(|v| {
            if let Scalar::Utf8(s) = v {
                for (t, r) in targets.iter().zip(repls) {
                    if s == t {
                        return Scalar::Utf8(r.clone());
                    }
                }
            }
            v.clone()
        })
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

    // k=10 targets (grows the O(N·k) advantage).
    let targets: Vec<String> = (0..10).map(|k| format!("cat_{:06}", k)).collect();
    let repls: Vec<String> = (0..10).map(|k| format!("R{k}")).collect();
    let t_sc: Vec<Scalar> = targets.iter().map(|s| Scalar::Utf8(s.clone())).collect();
    let r_sc: Vec<Scalar> = repls.iter().map(|s| Scalar::Utf8(s.clone())).collect();
    let warm_vals = materialize(&bytes, &offsets, &present);

    let mut best_t = u128::MAX;
    for _ in 0..iters {
        let t = std::time::Instant::now();
        let r = col.replace_values(&t_sc, &r_sc).unwrap();
        best_t = best_t.min(t.elapsed().as_nanos());
        std::hint::black_box(&r);
    }
    let mut best_cold = u128::MAX;
    for _ in 0..iters {
        let t = std::time::Instant::now();
        let vals = materialize(&bytes, &offsets, &present);
        let r = replace_scan(&vals, &targets, &repls);
        best_cold = best_cold.min(t.elapsed().as_nanos());
        std::hint::black_box(&r);
    }
    let mut best_warm = u128::MAX;
    for _ in 0..iters {
        let t = std::time::Instant::now();
        let r = replace_scan(&warm_vals, &targets, &repls);
        best_warm = best_warm.min(t.elapsed().as_nanos());
        std::hint::black_box(&r);
    }
    let got = col.replace_values(&t_sc, &r_sc).unwrap();
    let want = replace_scan(&warm_vals, &targets, &repls);
    let gv = got.values();
    let wv = want.values();
    assert_eq!(gv.len(), wv.len());
    for k in 0..n {
        assert_eq!(format!("{:?}", gv.get(k)), format!("{:?}", wv.get(k)), "slot {k}");
    }
    println!(
        "replace utf8_nullable k=10 n={n} NEW={:>7.2}ms COLD={:>7.2}ms(={:.2}x) WARM={:>7.2}ms(={:.2}x)",
        best_t as f64 / 1e6,
        best_cold as f64 / 1e6,
        best_cold as f64 / best_t as f64,
        best_warm as f64 / 1e6,
        best_warm as f64 / best_t as f64,
    );
}
