//! Bench: nullable-Int64 Column::filter_by_mask (boolean indexing df[mask]). The typed arm
//! gated on all-valid, so a nullable Int64 column fell to the generic Vec<Scalar> gather.
//! NEW = compact raw i64 + validity bits at mask-true positions.
//! Run: cargo run -p fp-columnar --release --example bench_filter_i64null -- 5000000 20

use fp_columnar::{Column, ValidityMask};
use fp_types::{NullKind, Scalar};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let n: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(5_000_000);
    let iters: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(20);

    let mut state: u64 = 0xF117E5;
    let mut data = vec![0i64; n];
    let mut validity = ValidityMask::all_valid(n);
    let mut mask_sc: Vec<Scalar> = Vec::with_capacity(n);
    let mut col_sc: Vec<Scalar> = Vec::with_capacity(n);
    for i in 0..n {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
        if (state >> 40) % 3 == 0 {
            validity.set(i, false);
            col_sc.push(Scalar::Null(NullKind::Null));
        } else {
            data[i] = ((state >> 20) % 1_000_000) as i64;
            col_sc.push(Scalar::Int64(data[i]));
        }
        mask_sc.push(Scalar::Bool((state >> 33) & 1 == 0));
    }
    let col = Column::from_i64_values_with_validity(data, validity);
    let col_eager = Column::new(fp_types::DType::Int64, col_sc).unwrap();
    let mask = Column::from_bool_values(mask_sc.iter().map(|m| matches!(m, Scalar::Bool(true))).collect());

    let got = col.filter_by_mask(&mask).unwrap();
    assert_eq!(got.dtype(), fp_types::DType::Int64);
    assert_eq!(got.values(), col_eager.filter_by_mask(&mask).unwrap().values());
    println!("filter_by_mask(nullable Int64) OK, out.len()={}", got.len());

    let mut best_new = u128::MAX;
    for _ in 0..iters {
        let t = std::time::Instant::now();
        let o = col.filter_by_mask(&mask).unwrap();
        best_new = best_new.min(t.elapsed().as_nanos());
        std::hint::black_box(&o);
    }
    let mut best_ref = u128::MAX;
    for _ in 0..iters {
        let t = std::time::Instant::now();
        let o = col_eager.filter_by_mask(&mask).unwrap();
        best_ref = best_ref.min(t.elapsed().as_nanos());
        std::hint::black_box(&o);
    }
    println!(
        "filter_by_mask nullable-Int64 n={n} NEW={:.2}ms GENERIC={:.2}ms ({:.1}x)",
        best_new as f64 / 1e6,
        best_ref as f64 / 1e6,
        best_ref as f64 / best_new as f64,
    );
}
