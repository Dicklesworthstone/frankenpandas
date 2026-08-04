//! Bench: nullable-Int64 Column::reindex_by_positions (align/reindex/outer-join null-fill).
//! The null-introducing typed arms gated on validity.all(), so a nullable Int64 source fell
//! to the generic Vec<Scalar> gather. NEW = variant-gated raw i64 + validity gather.
//! Run: cargo run -p fp-columnar --release --example bench_reindex_i64null -- 5000000 20

use fp_columnar::{Column, ValidityMask};
use fp_types::{NullKind, Scalar};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let n: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(5_000_000);
    let iters: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(20);

    let mut state: u64 = 0x2E1DE5;
    let mut data = vec![0i64; n];
    let mut validity = ValidityMask::all_valid(n);
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
    }
    let col = Column::from_i64_values_with_validity(data, validity);
    let col_eager = Column::new(fp_types::DType::Int64, col_sc).unwrap();

    // Positions: ~30% None (null-introducing), rest random present.
    let positions: Vec<Option<usize>> = (0..n)
        .map(|_| {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
            if (state >> 41) % 10 < 3 {
                None
            } else {
                Some(((state >> 15) % n as u64) as usize)
            }
        })
        .collect();

    let got = col.reindex_by_positions(&positions).unwrap();
    assert_eq!(got.dtype(), fp_types::DType::Int64);
    assert_eq!(got.values(), col_eager.reindex_by_positions(&positions).unwrap().values());
    println!("reindex_by_positions(nullable Int64) OK, len={}", got.len());

    let mut best_new = u128::MAX;
    for _ in 0..iters {
        let t = std::time::Instant::now();
        let o = col.reindex_by_positions(&positions).unwrap();
        best_new = best_new.min(t.elapsed().as_nanos());
        std::hint::black_box(&o);
    }
    let mut best_ref = u128::MAX;
    for _ in 0..iters {
        let t = std::time::Instant::now();
        let o = col_eager.reindex_by_positions(&positions).unwrap();
        best_ref = best_ref.min(t.elapsed().as_nanos());
        std::hint::black_box(&o);
    }
    println!(
        "reindex nullable-Int64 n={n} NEW={:.2}ms GENERIC={:.2}ms ({:.1}x)",
        best_new as f64 / 1e6,
        best_ref as f64 / 1e6,
        best_ref as f64 / best_new as f64,
    );
}
