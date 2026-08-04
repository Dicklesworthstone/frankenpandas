//! Bench: nullable-Int64 Column::isin (df["id"].isin(ids)). The typed membership arms gated
//! on all-valid, so a nullable Int64 column fell to the generic Vec<Scalar> + per-row key
//! probe. NEW = raw i64 + validity probed against an FxHashSet<i64>.
//! Run: cargo run -p fp-columnar --release --example bench_isin_i64null -- 5000000 20

use std::collections::HashSet;

use fp_columnar::{Column, ValidityMask};
use fp_types::{NullKind, Scalar};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let n: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(5_000_000);
    let iters: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(20);

    let mut state: u64 = 0x1519E5;
    let mut data = vec![0i64; n];
    let mut validity = ValidityMask::all_valid(n);
    let mut col_sc: Vec<Scalar> = Vec::with_capacity(n);
    for i in 0..n {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
        if (state >> 40) % 3 == 0 {
            validity.set(i, false);
            col_sc.push(Scalar::Null(NullKind::Null));
        } else {
            data[i] = ((state >> 20) % 1_000) as i64;
            col_sc.push(Scalar::Int64(data[i]));
        }
    }
    let col = Column::from_i64_values_with_validity(data.clone(), validity.clone());
    // Needle set: ~200 of the 1000 possible values (wide-ish membership).
    let needles: Vec<Scalar> = (0..200).map(|k| Scalar::Int64(k * 5)).collect();
    let needle_ints: HashSet<i64> = (0..200).map(|k| k * 5).collect();

    let got = col.isin(&needles).unwrap();
    assert_eq!(got.dtype(), fp_types::DType::Bool);
    println!("isin(nullable Int64) OK");

    let mut best_new = u128::MAX;
    for _ in 0..iters {
        let t = std::time::Instant::now();
        let o = col.isin(&needles).unwrap();
        best_new = best_new.min(t.elapsed().as_nanos());
        std::hint::black_box(&o);
    }
    // Reference = the OLD generic shape on a COLD column: materialize the Vec<Scalar> each
    // call (the `df["id"].isin(ids)` fresh-column shape), then per-row membership probe.
    let mut best_ref = u128::MAX;
    for _ in 0..iters {
        let t = std::time::Instant::now();
        let scalars: Vec<Scalar> = (0..n)
            .map(|i| {
                if validity.get(i) {
                    Scalar::Int64(data[i])
                } else {
                    Scalar::Null(NullKind::Null)
                }
            })
            .collect();
        let out: Vec<bool> = scalars
            .iter()
            .map(|s| match s {
                Scalar::Int64(v) => needle_ints.contains(v),
                _ => false,
            })
            .collect();
        let o = Column::from_bool_values(out);
        best_ref = best_ref.min(t.elapsed().as_nanos());
        std::hint::black_box(&o);
    }
    let _ = &col_sc;
    println!(
        "isin nullable-Int64 n={n} NEW={:.2}ms REF(materialize)={:.2}ms ({:.1}x)",
        best_new as f64 / 1e6,
        best_ref as f64 / 1e6,
        best_ref as f64 / best_new as f64,
    );
}
