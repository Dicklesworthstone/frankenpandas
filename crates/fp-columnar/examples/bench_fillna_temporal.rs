//! Bench: nullable-temporal Column::fillna. A nullable Datetime64/Timedelta64 column with
//! a temporal fill scalar had NO typed arm → the generic loop materialized a Vec<Scalar>
//! (present clones + fill) + Self::new rescan. NEW = typed i64-ns fill → all-valid buffer.
//! Run: cargo run -p fp-columnar --release --example bench_fillna_temporal -- 5000000 20

use fp_columnar::{Column, ValidityMask};
use fp_types::{NullKind, Scalar};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let n: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(5_000_000);
    let iters: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(20);

    let base: i64 = 1_600_000_000_000_000_000;
    let mut state: u64 = 0xF111_5EED;
    let mut data = vec![0i64; n];
    let mut validity = ValidityMask::all_valid(n);
    for i in 0..n {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
        if (state >> 40) % 4 == 0 {
            validity.set(i, false);
        } else {
            data[i] = base + ((state >> 20) % 10_000_000) as i64;
        }
    }
    let col = Column::from_datetime64_values_with_validity(data.clone(), validity.clone());
    let fill = Scalar::Datetime64(base);

    // Correctness spot check.
    let got = col.fillna(&fill).unwrap();
    assert_eq!(got.dtype(), fp_types::DType::Datetime64);
    for i in 0..n.min(1000) {
        let want = if validity.get(i) { data[i] } else { base };
        assert_eq!(got.values()[i], Scalar::Datetime64(want), "row {i}");
    }
    println!("fillna(Datetime64) OK, all-valid out = {}", !got.has_nulls());

    // Reference = the OLD generic path: materialize Vec<Scalar>, clone present / fill missing.
    let scalars: Vec<Scalar> = (0..n)
        .map(|i| {
            if validity.get(i) {
                Scalar::Datetime64(data[i])
            } else {
                Scalar::Null(NullKind::NaT)
            }
        })
        .collect();

    let mut best_new = u128::MAX;
    for _ in 0..iters {
        let t = std::time::Instant::now();
        let o = col.fillna(&fill).unwrap();
        best_new = best_new.min(t.elapsed().as_nanos());
        std::hint::black_box(&o);
    }
    let mut best_ref = u128::MAX;
    for _ in 0..iters {
        let t = std::time::Instant::now();
        let out: Vec<Scalar> = scalars
            .iter()
            .map(|v| if v.is_missing() { fill.clone() } else { v.clone() })
            .collect();
        let o = Column::from_values(out).unwrap();
        best_ref = best_ref.min(t.elapsed().as_nanos());
        std::hint::black_box(&o);
    }
    println!(
        "fillna nullable-Datetime64 n={n} NEW={:.2}ms REF(materialize)={:.2}ms ({:.1}x)",
        best_new as f64 / 1e6,
        best_ref as f64 / 1e6,
        best_ref as f64 / best_new as f64,
    );
}
